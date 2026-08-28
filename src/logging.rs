//! Best-effort structured local diagnostics.

use std::fs::{self, OpenOptions};
use std::io::Write;

use chrono::Utc;
use serde_json::{Map, Value, json};

pub fn log_event(event: &str, attributes: impl IntoIterator<Item = (String, Value)>) {
    let Some(home) = home::home_dir() else {
        return;
    };
    let now = Utc::now();
    let target = home
        .join(".kestrel")
        .join("logs")
        .join(now.format("%Y-%m-%d").to_string())
        .join("events.jsonl");
    let mut record = Map::new();
    record.insert("timestamp".into(), json!(now.to_rfc3339()));
    record.insert("event".into(), json!(event));
    record.extend(attributes);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(target)?;
        writeln!(file, "{}", Value::Object(record))
    })();
    let _ = result;
}

#[macro_export]
macro_rules! log_event {
    ($event:expr $(, $key:expr => $value:expr)* $(,)?) => {{
        $crate::logging::log_event(
            $event,
            vec![$(($key.to_owned(), serde_json::json!($value))),*],
        )
    }};
}
