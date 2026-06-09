use crate::har::types::{
    AnalysisSession, AppSettings, ChatMessage, HarChunk, HarEntryDetail, HarEntrySummary,
    HeaderPair,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    file_path TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    total_entries INTEGER NOT NULL,
                    total_bytes INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    status TEXT NOT NULL,
                    final_summary TEXT
                );

                CREATE TABLE IF NOT EXISTS entries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    entry_index INTEGER NOT NULL,
                    method TEXT NOT NULL,
                    url TEXT NOT NULL,
                    status INTEGER NOT NULL,
                    mime_type TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    time_ms REAL NOT NULL,
                    started_at TEXT,
                    request_headers TEXT DEFAULT '[]',
                    response_headers TEXT DEFAULT '[]',
                    request_body TEXT DEFAULT '',
                    response_body TEXT DEFAULT '',
                    js_insights TEXT DEFAULT '[]',
                    is_javascript INTEGER DEFAULT 0,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS chunks (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    chunk_index INTEGER NOT NULL,
                    entry_count INTEGER NOT NULL,
                    estimated_tokens INTEGER NOT NULL,
                    payload TEXT NOT NULL,
                    summary TEXT,
                    status TEXT NOT NULL,
                    chunk_type TEXT DEFAULT 'traffic',
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS chat_messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    context_type TEXT,
                    context_ref TEXT,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_entries_session ON entries(session_id);
                CREATE INDEX IF NOT EXISTS idx_chunks_session ON chunks(session_id);
                CREATE INDEX IF NOT EXISTS idx_chat_session ON chat_messages(session_id);
                ",
            )
            .map_err(|e| e.to_string())?;

        self.migrate()?;
        Ok(())
    }

    fn migrate(&self) -> Result<(), String> {
        let alters = [
            "ALTER TABLE entries ADD COLUMN request_headers TEXT DEFAULT '[]'",
            "ALTER TABLE entries ADD COLUMN response_headers TEXT DEFAULT '[]'",
            "ALTER TABLE entries ADD COLUMN request_body TEXT DEFAULT ''",
            "ALTER TABLE entries ADD COLUMN response_body TEXT DEFAULT ''",
            "ALTER TABLE entries ADD COLUMN js_insights TEXT DEFAULT '[]'",
            "ALTER TABLE entries ADD COLUMN is_javascript INTEGER DEFAULT 0",
            "ALTER TABLE chunks ADD COLUMN chunk_type TEXT DEFAULT 'traffic'",
            "ALTER TABLE entries ADD COLUMN resource_type TEXT DEFAULT ''",
        ];
        for sql in alters {
            let _ = self.conn.execute(sql, []);
        }
        Ok(())
    }

    pub fn get_settings(&self) -> Result<AppSettings, String> {
        let mut settings = AppSettings::default();
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM settings")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;

        for row in rows {
            let (key, value) = row.map_err(|e| e.to_string())?;
            match key.as_str() {
                "openrouter_api_key" => settings.openrouter_api_key = value,
                "default_model" => settings.default_model = value,
                "thinking_model" => settings.thinking_model = value,
                "chunk_max_tokens" => {
                    settings.chunk_max_tokens = value.parse().unwrap_or(3000);
                }
                "filter_static_assets" => {
                    settings.filter_static_assets = value == "true";
                }
                "max_concurrent_requests" => {
                    settings.max_concurrent_requests = value.parse().unwrap_or(4).clamp(1, 16);
                }
                "analyze_javascript" => {
                    settings.analyze_javascript = value != "false";
                }
                "chat_agent_max_steps" => {
                    settings.chat_agent_max_steps = value.parse().unwrap_or(10).clamp(1, 50);
                }
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let chunk_tokens = settings.chunk_max_tokens.to_string();
        let max_concurrent = settings.max_concurrent_requests.to_string();
        let chat_agent_steps = settings.chat_agent_max_steps.clamp(1, 50).to_string();
        let pairs: [(&str, &str); 8] = [
            ("openrouter_api_key", settings.openrouter_api_key.as_str()),
            ("default_model", settings.default_model.as_str()),
            ("thinking_model", settings.thinking_model.as_str()),
            ("chunk_max_tokens", &chunk_tokens),
            (
                "filter_static_assets",
                if settings.filter_static_assets {
                    "true"
                } else {
                    "false"
                },
            ),
            ("max_concurrent_requests", &max_concurrent),
            (
                "analyze_javascript",
                if settings.analyze_javascript {
                    "true"
                } else {
                    "false"
                },
            ),
            ("chat_agent_max_steps", &chat_agent_steps),
        ];

        for (key, value) in pairs {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn create_session(
        &self,
        file_path: &str,
        file_name: &str,
        total_bytes: u64,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO sessions (id, file_path, file_name, total_entries, total_bytes, created_at, status) VALUES (?1, ?2, ?3, 0, ?4, ?5, 'parsing')",
                params![id, file_path, file_name, total_bytes as i64, created_at],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn insert_entries(&self, session_id: &str, entries: &[HarEntryDetail]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO entries (session_id, entry_index, method, url, status, mime_type, size, time_ms, started_at, request_headers, response_headers, request_body, response_body, js_insights, is_javascript, resource_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                )
                .map_err(|e| e.to_string())?;

            for entry in entries {
                let s = &entry.summary;
                stmt.execute(params![
                    session_id,
                    s.index as i64,
                    s.method,
                    s.url,
                    s.status as i64,
                    s.mime_type,
                    s.size as i64,
                    s.time_ms,
                    s.started_at,
                    serde_json::to_string(&entry.request_headers).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&entry.response_headers).unwrap_or_else(|_| "[]".into()),
                    entry.request_body,
                    entry.response_body,
                    serde_json::to_string(&entry.js_insights).unwrap_or_else(|_| "[]".into()),
                    s.is_javascript as i64,
                    s.resource_type.clone().unwrap_or_default(),
                ])
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;

        self.conn
            .execute(
                "UPDATE sessions SET total_entries = ?1, status = 'parsed' WHERE id = ?2",
                params![entries.len() as i64, session_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn insert_chunks(&self, chunks: &[HarChunk]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO chunks (id, session_id, chunk_index, entry_count, estimated_tokens, payload, summary, status, chunk_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|e| e.to_string())?;

            for chunk in chunks {
                stmt.execute(params![
                    chunk.id,
                    chunk.session_id,
                    chunk.chunk_index as i64,
                    chunk.entry_count as i64,
                    chunk.estimated_tokens as i64,
                    chunk.payload,
                    chunk.summary,
                    chunk.status,
                    chunk.chunk_type,
                ])
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<AnalysisSession>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, file_name, total_entries, total_bytes, created_at, status, final_summary FROM sessions ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                Ok(AnalysisSession {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                    total_entries: row.get::<_, i64>(3)? as usize,
                    total_bytes: row.get::<_, i64>(4)? as u64,
                    created_at: row.get(5)?,
                    status: row.get(6)?,
                    final_summary: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<AnalysisSession>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, file_name, total_entries, total_bytes, created_at, status, final_summary FROM sessions WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt
            .query(params![session_id])
            .map_err(|e| e.to_string())?;

        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            Ok(Some(AnalysisSession {
                id: row.get(0).map_err(|e| e.to_string())?,
                file_path: row.get(1).map_err(|e| e.to_string())?,
                file_name: row.get(2).map_err(|e| e.to_string())?,
                total_entries: row.get::<_, i64>(3).map_err(|e| e.to_string())? as usize,
                total_bytes: row.get::<_, i64>(4).map_err(|e| e.to_string())? as u64,
                created_at: row.get(5).map_err(|e| e.to_string())?,
                status: row.get(6).map_err(|e| e.to_string())?,
                final_summary: row.get(7).map_err(|e| e.to_string())?,
            }))
        } else {
            Ok(None)
        }
    }

    fn row_to_summary(row: &rusqlite::Row) -> rusqlite::Result<HarEntrySummary> {
        let resource_type: String = row.get(9)?;
        Ok(HarEntrySummary {
            index: row.get::<_, i64>(0)? as usize,
            method: row.get(1)?,
            url: row.get(2)?,
            status: row.get::<_, i64>(3)? as u16,
            mime_type: row.get(4)?,
            size: row.get::<_, i64>(5)? as u64,
            time_ms: row.get(6)?,
            started_at: row.get(7)?,
            is_javascript: row.get::<_, i64>(8)? != 0,
            resource_type: if resource_type.is_empty() {
                None
            } else {
                Some(resource_type)
            },
        })
    }

    pub fn get_session_entries(&self, session_id: &str) -> Result<Vec<HarEntrySummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_index, method, url, status, mime_type, size, time_ms, started_at, is_javascript, resource_type FROM entries WHERE session_id = ?1 ORDER BY entry_index",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![session_id], Self::row_to_summary)
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn search_entries(
        &self,
        session_id: &str,
        query: Option<&str>,
        method: Option<&str>,
        status_min: Option<u16>,
        status_max: Option<u16>,
        js_only: bool,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<HarEntrySummary>, String> {
        let (sql, params) = Self::entry_filter_query(
            session_id,
            query,
            method,
            status_min,
            status_max,
            js_only,
            true,
        );

        let sql = format!(
            "{sql} ORDER BY entry_index LIMIT ?{} OFFSET ?{}",
            params.len() + 1,
            params.len() + 2
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        Self::query_summaries(&mut stmt, &params, limit as i64, offset as i64)
    }

    pub fn count_entries(
        &self,
        session_id: &str,
        query: Option<&str>,
        method: Option<&str>,
        status_min: Option<u16>,
        status_max: Option<u16>,
        js_only: bool,
    ) -> Result<usize, String> {
        let (sql, params) = Self::entry_filter_query(
            session_id,
            query,
            method,
            status_min,
            status_max,
            js_only,
            false,
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let count = Self::query_scalar(&mut stmt, &params)?;
        Ok(count as usize)
    }

    fn entry_filter_query(
        session_id: &str,
        query: Option<&str>,
        method: Option<&str>,
        status_min: Option<u16>,
        status_max: Option<u16>,
        js_only: bool,
        select_rows: bool,
    ) -> (String, Vec<rusqlite::types::Value>) {
        let mut sql = if select_rows {
            String::from(
                "SELECT entry_index, method, url, status, mime_type, size, time_ms, started_at, is_javascript, resource_type FROM entries WHERE session_id = ?1",
            )
        } else {
            String::from("SELECT COUNT(*) FROM entries WHERE session_id = ?1")
        };

        let mut params: Vec<rusqlite::types::Value> = vec![session_id.to_string().into()];

        if let Some(q) = query.filter(|s| !s.is_empty()) {
            params.push(format!("%{q}%").into());
            sql.push_str(&format!(" AND LOWER(url) LIKE LOWER(?{})", params.len()));
        }

        if let Some(m) = method.filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all")) {
            params.push(m.to_ascii_uppercase().into());
            sql.push_str(&format!(" AND UPPER(method) = ?{}", params.len()));
        }

        if let Some(min) = status_min {
            params.push(i64::from(min).into());
            sql.push_str(&format!(" AND status >= ?{}", params.len()));
        }

        if let Some(max) = status_max {
            params.push(i64::from(max).into());
            sql.push_str(&format!(" AND status <= ?{}", params.len()));
        }

        if js_only {
            sql.push_str(" AND is_javascript = 1");
        }

        (sql, params)
    }

    fn query_summaries(
        stmt: &mut rusqlite::Statement,
        params: &[rusqlite::types::Value],
        limit: i64,
        offset: i64,
    ) -> Result<Vec<HarEntrySummary>, String> {
        let mut bound: Vec<&dyn rusqlite::ToSql> = params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        bound.push(&limit);
        bound.push(&offset);

        let rows = stmt
            .query_map(bound.as_slice(), Self::row_to_summary)
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    fn query_scalar(
        stmt: &mut rusqlite::Statement,
        params: &[rusqlite::types::Value],
    ) -> Result<i64, String> {
        let bound: Vec<&dyn rusqlite::ToSql> = params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        stmt.query_row(bound.as_slice(), |row| row.get(0))
            .map_err(|e| e.to_string())
    }

    pub fn get_entry_detail(
        &self,
        session_id: &str,
        entry_index: usize,
    ) -> Result<Option<HarEntryDetail>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_index, method, url, status, mime_type, size, time_ms, started_at, is_javascript, resource_type, request_headers, response_headers, request_body, response_body, js_insights FROM entries WHERE session_id = ?1 AND entry_index = ?2",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt
            .query(params![session_id, entry_index as i64])
            .map_err(|e| e.to_string())?;

        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let request_headers: Vec<HeaderPair> =
                serde_json::from_str(&row.get::<_, String>(10).map_err(|e| e.to_string())?)
                    .unwrap_or_default();
            let response_headers: Vec<HeaderPair> =
                serde_json::from_str(&row.get::<_, String>(11).map_err(|e| e.to_string())?)
                    .unwrap_or_default();
            let js_insights: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(14).map_err(|e| e.to_string())?)
                    .unwrap_or_default();
            let resource_type: String = row.get(9).map_err(|e| e.to_string())?;

            Ok(Some(HarEntryDetail {
                summary: HarEntrySummary {
                    index: row.get::<_, i64>(0).map_err(|e| e.to_string())? as usize,
                    method: row.get(1).map_err(|e| e.to_string())?,
                    url: row.get(2).map_err(|e| e.to_string())?,
                    status: row.get::<_, i64>(3).map_err(|e| e.to_string())? as u16,
                    mime_type: row.get(4).map_err(|e| e.to_string())?,
                    size: row.get::<_, i64>(5).map_err(|e| e.to_string())? as u64,
                    time_ms: row.get(6).map_err(|e| e.to_string())?,
                    started_at: row.get(7).map_err(|e| e.to_string())?,
                    is_javascript: row.get::<_, i64>(8).map_err(|e| e.to_string())? != 0,
                    resource_type: if resource_type.is_empty() {
                        None
                    } else {
                        Some(resource_type)
                    },
                },
                request_headers,
                response_headers,
                request_body: row.get(12).map_err(|e| e.to_string())?,
                response_body: row.get(13).map_err(|e| e.to_string())?,
                js_insights,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_session_entry_details(&self, session_id: &str) -> Result<Vec<HarEntryDetail>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_index, method, url, status, mime_type, size, time_ms, started_at, is_javascript, resource_type, request_headers, response_headers, request_body, response_body, js_insights FROM entries WHERE session_id = ?1 ORDER BY entry_index",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                let request_headers: Vec<HeaderPair> =
                    serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default();
                let response_headers: Vec<HeaderPair> =
                    serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
                let js_insights: Vec<String> =
                    serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default();
                let resource_type: String = row.get(9)?;

                Ok(HarEntryDetail {
                    summary: HarEntrySummary {
                        index: row.get::<_, i64>(0)? as usize,
                        method: row.get(1)?,
                        url: row.get(2)?,
                        status: row.get::<_, i64>(3)? as u16,
                        mime_type: row.get(4)?,
                        size: row.get::<_, i64>(5)? as u64,
                        time_ms: row.get(6)?,
                        started_at: row.get(7)?,
                        is_javascript: row.get::<_, i64>(8)? != 0,
                        resource_type: if resource_type.is_empty() {
                            None
                        } else {
                            Some(resource_type)
                        },
                    },
                    request_headers,
                    response_headers,
                    request_body: row.get(12)?,
                    response_body: row.get(13)?,
                    js_insights,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_session_chunks(&self, session_id: &str) -> Result<Vec<HarChunk>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, chunk_index, entry_count, estimated_tokens, payload, summary, status, chunk_type FROM chunks WHERE session_id = ?1 ORDER BY chunk_index",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(HarChunk {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    chunk_index: row.get::<_, i64>(2)? as usize,
                    entry_count: row.get::<_, i64>(3)? as usize,
                    estimated_tokens: row.get::<_, i64>(4)? as usize,
                    payload: row.get(5)?,
                    summary: row.get(6)?,
                    status: row.get(7)?,
                    chunk_type: row.get::<_, String>(8).unwrap_or_else(|_| "traffic".into()),
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn update_chunk_summary(
        &self,
        chunk_id: &str,
        summary: &str,
        status: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE chunks SET summary = ?1, status = ?2 WHERE id = ?3",
                params![summary, status, chunk_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_session_status(&self, session_id: &str, status: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET status = ?1 WHERE id = ?2",
                params![status, session_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_session_summary(&self, session_id: &str, summary: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET final_summary = ?1, status = 'complete' WHERE id = ?2",
                params![summary, session_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM chat_messages WHERE session_id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute("DELETE FROM chunks WHERE session_id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute("DELETE FROM entries WHERE session_id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn clear_chunks(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM chunks WHERE session_id = ?1", params![session_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn reset_session_analysis(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE chunks SET summary = NULL, status = 'pending' WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "UPDATE sessions SET final_summary = NULL, status = 'parsed' WHERE id = ?1",
                params![session_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn insert_chat_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        context_type: Option<&str>,
        context_ref: Option<&str>,
    ) -> Result<ChatMessage, String> {
        let created_at = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO chat_messages (session_id, role, content, context_type, context_ref, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![session_id, role, content, context_type, context_ref, created_at],
            )
            .map_err(|e| e.to_string())?;
        let id = self.conn.last_insert_rowid();
        Ok(ChatMessage {
            id,
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            context_type: context_type.map(String::from),
            context_ref: context_ref.map(String::from),
            created_at,
        })
    }

    pub fn get_chat_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, role, content, context_type, context_ref, created_at FROM chat_messages WHERE session_id = ?1 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    context_type: row.get(4)?,
                    context_ref: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn clear_chat_messages(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM chat_messages WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
