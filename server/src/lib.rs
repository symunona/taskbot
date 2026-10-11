pub mod agents;
pub mod compact;
pub mod config;
pub mod net;
pub mod outline;
pub mod server;
pub mod state;
pub mod tasks;
pub mod tools;
pub mod vault;

/// Seconds since the unix epoch.
pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Milliseconds since the unix epoch.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// RFC 3339 UTC rendering of a unix-seconds timestamp.
pub fn iso(secs: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("OS random source unavailable");
    b
}
