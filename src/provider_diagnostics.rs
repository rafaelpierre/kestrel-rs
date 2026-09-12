//! Internal, versioned observations. Public result JSON is intentionally unchanged.
use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    NotStarted,
    Queue,
    Send,
    Body,
    Parse,
    Processing,
    Backoff,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Interval {
    pub phase: Phase,
    pub attempt_id: Option<String>,
    pub elapsed_ms: u64,
    pub censored: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Challenge {
    Detected,
    NotDetected,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransportKind {
    Timeout,
    Connect,
    Dns,
    Tls,
    Body,
    Decode,
    Request,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Attempt {
    pub run_id: String,
    pub search_id: String,
    pub attempt_id: String,
    pub ordinal: usize,
    pub http_status: Option<u16>,
    pub retry_after: Option<String>,
    pub challenge: Challenge,
    pub transport_error: Option<TransportKind>,
    pub outcome: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Lifecycle {
    pub schema_version: u32,
    pub run_id: String,
    pub benchmark_run_id: Option<String>,
    pub search_id: String,
    pub send_attempts: usize,
    pub count_unit: &'static str,
    pub redirect_hops_observed: bool,
    pub cancellation_phase: Option<Phase>,
    pub logical_outcome: Option<String>,
    pub attempts: Vec<Attempt>,
    pub intervals: Vec<Interval>,
}

#[derive(Debug)]
pub(crate) struct Recorder {
    snapshot: Lifecycle,
    current: Option<(Phase, Instant)>,
}

impl Recorder {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_run(uuid::Uuid::new_v4().to_string())
    }

    pub fn with_run(run_id: String) -> Self {
        Self {
            snapshot: Lifecycle {
                schema_version: 1,
                run_id,
                benchmark_run_id: std::env::var("KESTRELSEARCH_BENCHMARK_RUN_ID").ok(),
                search_id: uuid::Uuid::new_v4().to_string(),
                send_attempts: 0,
                count_unit: "application_send",
                redirect_hops_observed: false,
                cancellation_phase: None,
                logical_outcome: None,
                attempts: Vec::new(),
                intervals: Vec::new(),
            },
            current: Some((Phase::NotStarted, Instant::now())),
        }
    }

    pub fn transition(&mut self, phase: Phase) {
        self.transition_censored(phase, false);
    }

    pub fn transition_censored(&mut self, phase: Phase, censored: bool) {
        self.close(censored);
        self.current = Some((phase, Instant::now()));
    }

    pub fn start_attempt(&mut self) {
        // Close backoff/queue before assigning the next attempt ID.
        self.close(false);
        self.snapshot.send_attempts += 1;
        self.snapshot.attempts.push(Attempt {
            run_id: self.snapshot.run_id.clone(),
            search_id: self.snapshot.search_id.clone(),
            attempt_id: format!(
                "{}:{}",
                self.snapshot.search_id, self.snapshot.send_attempts
            ),
            ordinal: self.snapshot.send_attempts,
            http_status: None,
            retry_after: None,
            challenge: Challenge::Unknown,
            transport_error: None,
            outcome: None,
        });
        self.current = Some((Phase::Send, Instant::now()));
    }

    pub fn headers(&mut self, status: u16, retry_after: Option<String>) {
        if let Some(attempt) = self.snapshot.attempts.last_mut() {
            attempt.http_status = Some(status);
            attempt.retry_after = retry_after;
        }
    }

    pub fn response(&mut self, challenge: Challenge) {
        if let Some(attempt) = self.snapshot.attempts.last_mut() {
            attempt.challenge = challenge;
            attempt.outcome = Some("response");
        }
    }

    pub fn response_too_large(&mut self) {
        if let Some(attempt) = self.snapshot.attempts.last_mut() {
            attempt.outcome = Some("response_too_large");
        }
    }

    pub fn error(&mut self, kind: TransportKind, body: bool) {
        if let Some(attempt) = self.snapshot.attempts.last_mut() {
            attempt.transport_error = Some(kind);
            attempt.outcome = Some(if body {
                "body_error"
            } else {
                "transport_error"
            });
        }
    }

    pub fn correlation(&self) -> Option<serde_json::Value> {
        self.snapshot.attempts.last().map(|attempt| {
            serde_json::json!({
                "run_id": attempt.run_id, "search_id": attempt.search_id,
                "attempt_id": attempt.attempt_id,
                "benchmark_run_id": self.snapshot.benchmark_run_id,
            })
        })
    }

    pub fn finish(&mut self, cancelled: bool) -> Lifecycle {
        if self.current.is_some() {
            if cancelled {
                self.snapshot.cancellation_phase = self.current.map(|(phase, _)| phase);
            }
            if let Some(attempt) = self
                .snapshot
                .attempts
                .last_mut()
                .filter(|a| a.outcome.is_none())
            {
                attempt.outcome = Some(if cancelled { "cancelled" } else { "unknown" });
            }
            self.close(cancelled);
        }
        self.snapshot.clone()
    }

    pub fn finish_outcome(&mut self, outcome: &str, cancelled: bool) -> Lifecycle {
        if self.snapshot.logical_outcome.is_none() {
            self.snapshot.logical_outcome = Some(outcome.to_owned());
        }
        self.finish(cancelled)
    }

    pub fn retries(&self) -> usize {
        self.snapshot.send_attempts.saturating_sub(1)
    }

    fn close(&mut self, censored: bool) {
        if let Some((phase, started)) = self.current.take() {
            let attempt_id = if matches!(phase, Phase::NotStarted | Phase::Queue | Phase::Backoff) {
                None
            } else {
                self.snapshot.attempts.last().map(|a| a.attempt_id.clone())
            };
            self.snapshot.intervals.push(Interval {
                phase,
                attempt_id,
                elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                censored,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_cancellation_preserves_sends_and_completed_intervals() {
        let mut recorder = Recorder::new();
        recorder.start_attempt();
        recorder.transition(Phase::Body);
        recorder.headers(500, None);
        recorder.response(Challenge::NotDetected);
        recorder.transition(Phase::Backoff);
        recorder.start_attempt();
        recorder.error(TransportKind::Connect, false);
        recorder.transition(Phase::Backoff);
        let snapshot = recorder.finish(true);
        assert_eq!(snapshot.send_attempts, 2);
        assert_eq!(recorder.retries(), 1);
        assert_eq!(snapshot.cancellation_phase, Some(Phase::Backoff));
        assert!(snapshot.intervals.last().unwrap().censored);
        assert!(
            snapshot.intervals[..snapshot.intervals.len() - 1]
                .iter()
                .all(|i| !i.censored)
        );
        assert_eq!(
            recorder.finish(false).intervals.len(),
            snapshot.intervals.len()
        );
    }

    #[test]
    fn queued_cancellation_is_not_a_send() {
        let mut recorder = Recorder::new();
        recorder.transition(Phase::Queue);
        let snapshot = recorder.finish(true);
        assert_eq!(snapshot.send_attempts, 0);
        assert_eq!(recorder.retries(), 0);
        assert_eq!(snapshot.cancellation_phase, Some(Phase::Queue));
        assert!(snapshot.intervals.last().unwrap().censored);
    }

    #[test]
    fn completed_search_has_no_censored_intervals() {
        let mut recorder = Recorder::new();
        recorder.start_attempt();
        recorder.transition(Phase::Body);
        recorder.transition(Phase::Parse);
        let snapshot = recorder.finish(false);
        assert_eq!(snapshot.send_attempts, 1);
        assert_eq!(snapshot.cancellation_phase, None);
        assert!(snapshot.intervals.iter().all(|i| !i.censored));
        // Finalization is idempotent, even if a later caller requests cancellation.
        assert_eq!(recorder.finish(true).cancellation_phase, None);
    }
}
