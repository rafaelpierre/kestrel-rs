//! Internal failures retain semantic identity until the public error boundary.
use super::{KestrelError, ProviderOutcome};

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub(crate) struct ProviderFailure {
    pub(super) kind: ProviderOutcome,
    error: KestrelError,
}
impl ProviderFailure {
    pub(crate) fn challenge(message: String) -> Self {
        Self {
            kind: ProviderOutcome::Challenge,
            error: KestrelError::Search(message),
        }
    }
    pub(crate) fn unrecognized(message: String) -> Self {
        Self {
            kind: ProviderOutcome::Unrecognized,
            error: KestrelError::Search(message),
        }
    }
    pub(super) fn into_public(self) -> KestrelError {
        self.error
    }
}
impl From<KestrelError> for ProviderFailure {
    fn from(error: KestrelError) -> Self {
        let kind = match error {
            KestrelError::SearchDeadline => ProviderOutcome::Deadline,
            KestrelError::ProviderResponseTooLarge { .. } => ProviderOutcome::ResponseTooLarge,
            _ => ProviderOutcome::RequestError,
        };
        Self { kind, error }
    }
}
impl From<reqwest::Error> for ProviderFailure {
    fn from(error: reqwest::Error) -> Self {
        KestrelError::from(error).into()
    }
}
impl From<primp::Error> for ProviderFailure {
    fn from(error: primp::Error) -> Self {
        KestrelError::from(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[tokio::test(start_paused = true)]
    async fn messages_and_report_edits_do_not_control_attempt_state() {
        let _telemetry = crate::telemetry::test_export_guard();
        for (failure, expected) in [
            (
                ProviderFailure::challenge("wording without classification keywords".into()),
                ProviderOutcome::Challenge,
            ),
            (
                ProviderFailure::unrecognized("new explanatory wording".into()),
                ProviderOutcome::Unrecognized,
            ),
            (
                KestrelError::Search("bot challenge; unrecognized search page; deadline".into())
                    .into(),
                ProviderOutcome::RequestError,
            ),
            (
                KestrelError::SearchDeadline.into(),
                ProviderOutcome::Deadline,
            ),
            (
                KestrelError::ProviderResponseTooLarge {
                    engine: Engine::Bing,
                    limit_bytes: 42,
                    status: 200,
                }
                .into(),
                ProviderOutcome::ResponseTooLarge,
            ),
        ] {
            let diagnostics = Arc::new(Mutex::new(Vec::new()));
            let states = Arc::new(Mutex::new(HashMap::new()));
            let message = failure.to_string();
            let result = ATTEMPT_STATES
                .scope(states.clone(), async {
                    run_one_job(
                        "q",
                        Engine::Bing,
                        Arc::new(Semaphore::new(1)),
                        diagnostics.clone(),
                        None,
                        None,
                        async { Err(failure) },
                    )
                    .await
                })
                .await;
            assert_eq!(result.unwrap_err().to_string(), message);
            assert_eq!(diagnostics.lock().unwrap()[0].outcome, expected.as_str());
            diagnostics.lock().unwrap()[0].outcome = "deadline".into();
            diagnostics.lock().unwrap()[0].error = Some("reworded report".into());
            assert_eq!(states.lock().unwrap()[&Engine::Bing], expected);
            assert_eq!(
                states.lock().unwrap()[&Engine::Bing].retryable(),
                expected == ProviderOutcome::Deadline
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_state_survives_recorder_replacement_and_partial_snapshots() {
        let _telemetry = crate::telemetry::test_export_guard();
        for (status, header, expected) in [
            (200, None, ProviderOutcome::Deadline),
            (429, None, ProviderOutcome::RateLimitedDeadline),
            (
                503,
                Some("invalid but observed".into()),
                ProviderOutcome::RateLimitedDeadline,
            ),
        ] {
            let diagnostics = Arc::new(Mutex::new(Vec::new()));
            let states = Arc::new(Mutex::new(HashMap::new()));
            ATTEMPT_STATES
                .scope(states.clone(), async {
                    let result = run_one_job(
                        "q",
                        Engine::Bing,
                        Arc::new(Semaphore::new(1)),
                        diagnostics.clone(),
                        Some(tokio::time::Instant::now() + Duration::from_secs(1)),
                        None,
                        async {
                            record_attempt();
                            record_headers(status, header);
                            PROVIDER_RECORDER.with(|recorder| {
                                *recorder.lock().unwrap() = Recorder::with_run("replacement".into())
                            });
                            // A streamed partial snapshot remains reportable at a deadline.
                            PROVIDER_DIAGNOSTIC.with(|(rows, index)| {
                                let mut rows = rows.lock().unwrap();
                                rows[*index].result_count = 1;
                                rows[*index].raw_result_count = 2;
                                rows[*index].filtered_count = 1;
                            });
                            std::future::pending::<Result<ProviderResponse, ProviderFailure>>()
                                .await
                        },
                    )
                    .await;
                    assert!(matches!(result, Err(KestrelError::SearchDeadline)));
                })
                .await;
            assert_eq!(states.lock().unwrap()[&Engine::Bing], expected);
            let rows = diagnostics.lock().unwrap();
            assert_eq!(rows[0].outcome, expected.as_str());
            assert_eq!(rows[0].result_count, 1);
            assert!(expected.retains_snapshot());
            assert!(!rows[0].success);
        }
    }

    #[tokio::test]
    async fn dropping_unpolled_provider_records_typed_cancellation() {
        let _telemetry = crate::telemetry::test_export_guard();
        for (reason, expected) in [
            (FANOUT_RUNNING, ProviderOutcome::CancelledCaller),
            (FANOUT_MIN_RESULTS, ProviderOutcome::CancelledMinResults),
        ] {
            let states = Arc::new(Mutex::new(HashMap::new()));
            ATTEMPT_STATES
                .scope(states.clone(), async {
                    let job = run_one_job(
                        "q",
                        Engine::Bing,
                        Arc::new(Semaphore::new(1)),
                        Arc::new(Mutex::new(Vec::new())),
                        None,
                        Some(Arc::new(AtomicU8::new(reason))),
                        std::future::pending::<Result<ProviderResponse, ProviderFailure>>(),
                    );
                    drop(job);
                })
                .await;
            assert_eq!(states.lock().unwrap()[&Engine::Bing], expected);
            assert!(!expected.retryable());
        }
    }
}
