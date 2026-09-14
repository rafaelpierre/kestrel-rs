//! Isolated process: configuring diagnostics must not affect other test binaries.
use kestrelsearch::diagnostic_sink::{self, Config, Stats};

#[tokio::test]
async fn invalid_policy_does_not_initialize_and_disabled_policy_is_once_only() {
    let error = diagnostic_sink::configure(Config {
        queue_capacity: 0,
        ..Config::default()
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    diagnostic_sink::configure(Config {
        enabled: false,
        ..Config::default()
    })
    .unwrap();
    kestrelsearch::log_event!("disabled_test", "example" => "no persistence");
    assert!(diagnostic_sink::flush(std::time::Duration::from_millis(10)).await);
    assert_eq!(diagnostic_sink::stats(), Stats::default());
    let error = diagnostic_sink::configure(Config::default()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
}
