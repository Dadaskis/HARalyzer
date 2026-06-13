use crate::har::types::{HarEntryDetail, HeaderPair};
use serde_json::{json, Value};

fn headers_to_har(headers: &[HeaderPair]) -> Vec<Value> {
    headers
        .iter()
        .map(|h| {
            json!({
                "name": h.name,
                "value": h.value
            })
        })
        .collect()
}

pub fn build_har_json(entries: &[HarEntryDetail], title: &str) -> Value {
    let log_entries: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let s = &entry.summary;
            json!({
                "startedDateTime": s.started_at.as_deref().unwrap_or("1970-01-01T00:00:00.000Z"),
                "time": s.time_ms,
                "request": {
                    "method": s.method,
                    "url": s.url,
                    "httpVersion": "HTTP/1.1",
                    "headers": headers_to_har(&entry.request_headers),
                    "queryString": [],
                    "cookies": [],
                    "headersSize": -1,
                    "bodySize": entry.request_body.len(),
                    "postData": if entry.request_body.is_empty() {
                        Value::Null
                    } else {
                        json!({
                            "mimeType": s.mime_type,
                            "text": entry.request_body
                        })
                    }
                },
                "response": {
                    "status": s.status,
                    "statusText": "",
                    "httpVersion": "HTTP/1.1",
                    "headers": headers_to_har(&entry.response_headers),
                    "cookies": [],
                    "content": {
                        "size": s.size,
                        "mimeType": s.mime_type,
                        "text": entry.response_body
                    },
                    "redirectURL": "",
                    "headersSize": -1,
                    "bodySize": entry.response_body.len()
                },
                "cache": {},
                "timings": {
                    "send": 0,
                    "wait": s.time_ms,
                    "receive": 0
                },
                "_resourceType": s.resource_type.as_deref().unwrap_or("")
            })
        })
        .collect();

    json!({
        "log": {
            "version": "1.2",
            "creator": {
                "name": "HARalyzer",
                "version": "1.1.0"
            },
            "pages": [{
                "startedDateTime": "1970-01-01T00:00:00.000Z",
                "id": "page_1",
                "title": title,
                "pageTimings": { "onContentLoad": -1, "onLoad": -1 }
            }],
            "entries": log_entries
        }
    })
}
