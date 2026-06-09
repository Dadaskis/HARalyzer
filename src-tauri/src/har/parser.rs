use crate::har::types::RawHarEntry;
use crate::har::types::{entry_from_raw, HarEntryDetail, HarParseProgress, should_keep_entry};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use tauri::{AppHandle, Emitter};

const READ_BUF_SIZE: usize = 64 * 1024;

pub fn stream_parse_har<F>(
    path: &Path,
    mut on_progress: F,
    filter_static: bool,
    analyze_js: bool,
) -> Result<Vec<HarEntryDetail>, String>
where
    F: FnMut(HarParseProgress),
{
    let total_bytes = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read file metadata: {e}"))?
        .len();

    let file = File::open(path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mut reader = BufReader::with_capacity(READ_BUF_SIZE, file);

    on_progress(HarParseProgress {
        bytes_read: 0,
        total_bytes,
        entries_parsed: 0,
        phase: "scanning".to_string(),
    });

    find_entries_array_start(&mut reader, total_bytes, &mut on_progress)?;

    let mut entries = Vec::new();
    let mut index = 0usize;

    loop {
        skip_whitespace(&mut reader)?;
        let pos = reader.stream_position().map_err(|e| e.to_string())?;

        let mut peek = [0u8; 1];
        match reader.read_exact(&mut peek) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Read error: {e}")),
        }

        if peek[0] == b']' {
            break;
        }
        if peek[0] == b',' {
            continue;
        }
        if peek[0] != b'{' {
            return Err(format!("Expected object start, found '{}'", peek[0] as char));
        }

        reader
            .seek(SeekFrom::Current(-1))
            .map_err(|e| e.to_string())?;

        let obj_json = read_balanced_object(&mut reader)?;

        let raw: RawHarEntry =
            serde_json::from_str(&obj_json).map_err(|e| format!("Failed to parse entry: {e}"))?;

        let detail = entry_from_raw(index, raw);

        if should_keep_entry(filter_static, &detail, analyze_js) {
            entries.push(detail);
            index += 1;
        }

        if index % 100 == 0 {
            on_progress(HarParseProgress {
                bytes_read: pos,
                total_bytes,
                entries_parsed: entries.len(),
                phase: "parsing".to_string(),
            });
        }
    }

    on_progress(HarParseProgress {
        bytes_read: total_bytes,
        total_bytes,
        entries_parsed: entries.len(),
        phase: "complete".to_string(),
    });

    Ok(entries)
}

pub fn stream_parse_har_with_events(
    app: &AppHandle,
    path: &Path,
    filter_static: bool,
    analyze_js: bool,
) -> Result<Vec<HarEntryDetail>, String> {
    stream_parse_har(
        path,
        |progress| {
            let _ = app.emit("har-parse-progress", progress);
        },
        filter_static,
        analyze_js,
    )
}

fn find_entries_array_start<R: Read + Seek, F: FnMut(HarParseProgress)>(
    reader: &mut R,
    total_bytes: u64,
    on_progress: &mut F,
) -> Result<(), String> {
    let mut buf = vec![0u8; READ_BUF_SIZE];
    let mut window = String::new();
    let mut file_offset: u64 = 0;

    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Could not find entries array in HAR file".to_string());
        }

        window.push_str(&String::from_utf8_lossy(&buf[..n]));
        file_offset += n as u64;

        if let Some(key_pos) = window.find("\"entries\"") {
            let after_key = &window[key_pos + 9..];
            if let Some(bracket_pos) = after_key.find('[') {
                let absolute = key_pos + 9 + bracket_pos + 1;
                let window_start = file_offset - window.len() as u64;
                let seek_to = window_start + absolute as u64;
                reader
                    .seek(SeekFrom::Start(seek_to))
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
        }

        if window.len() > 16384 {
            window = window[window.len() - 8192..].to_string();
        }

        on_progress(HarParseProgress {
            bytes_read: file_offset,
            total_bytes,
            entries_parsed: 0,
            phase: "scanning".to_string(),
        });
    }
}

fn skip_whitespace<R: Read + Seek>(reader: &mut R) -> Result<(), String> {
    loop {
        let mut b = [0u8; 1];
        match reader.read_exact(&mut b) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        if !b[0].is_ascii_whitespace() {
            reader
                .seek(SeekFrom::Current(-1))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
}

fn read_balanced_object<R: Read>(reader: &mut R) -> Result<String, String> {
    let mut result = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    loop {
        let mut b = [0u8; 1];
        reader.read_exact(&mut b).map_err(|e| e.to_string())?;
        let ch = b[0] as char;
        result.push(ch);

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(result);
                }
            }
            _ => {}
        }
    }
}
