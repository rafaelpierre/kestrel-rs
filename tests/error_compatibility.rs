use kestrelsearch::{Engine, KestrelError};

#[test]
fn error_reexports_preserve_type_variants_display_and_conversions() {
    fn legacy_path(error: kestrelsearch::search::KestrelError) -> KestrelError {
        error
    }
    let error = legacy_path(KestrelError::ProviderResponseTooLarge {
        engine: Engine::Bing,
        limit_bytes: 42,
        status: 200,
    });
    assert_eq!(
        error.to_string(),
        "bing response exceeds 42 decoded bytes (HTTP 200)"
    );
    assert!(matches!(
        error,
        kestrelsearch::search::KestrelError::ProviderResponseTooLarge {
            engine: Engine::Bing,
            limit_bytes: 42,
            status: 200
        }
    ));
    let io: kestrelsearch::search::KestrelError = std::io::Error::other("fixture").into();
    assert_eq!(legacy_path(io).to_string(), "I/O failed: fixture");
    assert_eq!(
        legacy_path(KestrelError::SearchDeadline).to_string(),
        "search deadline exceeded"
    );
}
