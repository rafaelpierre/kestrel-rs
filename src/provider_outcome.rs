//! Typed interpretation of the stable provider diagnostic vocabulary.
/// Provider attempt classification. Public reports retain their original string field.
/// Unrecognized report values map to `Unknown` without changing the stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderOutcome {
    /// The `results` diagnostic outcome.
    Results,
    /// The `empty` diagnostic outcome.
    Empty,
    /// The `filtered_empty` diagnostic outcome.
    FilteredEmpty,
    /// The `deadline` diagnostic outcome.
    Deadline,
    /// The `rate_limited_deadline` diagnostic outcome.
    RateLimitedDeadline,
    /// The `cancelled_min_results` diagnostic outcome.
    CancelledMinResults,
    /// The `cancelled_quorum` diagnostic outcome.
    CancelledQuorum,
    /// The `cancelled_caller` diagnostic outcome.
    CancelledCaller,
    /// The `response_too_large` diagnostic outcome.
    ResponseTooLarge,
    /// The `challenge` diagnostic outcome.
    Challenge,
    /// The `unrecognized` diagnostic outcome.
    Unrecognized,
    /// The `request_error` diagnostic outcome.
    RequestError,
    /// The `unknown` diagnostic outcome.
    Unknown,
}
impl ProviderOutcome {
    /// Interpret a serialized report value; unknown values are never retryable.
    pub fn from_report(value: &str) -> Self {
        match value {
            "results" => Self::Results,
            "empty" => Self::Empty,
            "filtered_empty" => Self::FilteredEmpty,
            "deadline" => Self::Deadline,
            "rate_limited_deadline" => Self::RateLimitedDeadline,
            "cancelled_min_results" => Self::CancelledMinResults,
            "cancelled_quorum" => Self::CancelledQuorum,
            "cancelled_caller" => Self::CancelledCaller,
            "response_too_large" => Self::ResponseTooLarge,
            "challenge" => Self::Challenge,
            "unrecognized" => Self::Unrecognized,
            "request_error" => Self::RequestError,
            "unknown" => Self::Unknown,
            _ => Self::Unknown,
        }
    }
    /// Stable diagnostic spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Results => "results",
            Self::Empty => "empty",
            Self::FilteredEmpty => "filtered_empty",
            Self::Deadline => "deadline",
            Self::RateLimitedDeadline => "rate_limited_deadline",
            Self::CancelledMinResults => "cancelled_min_results",
            Self::CancelledQuorum => "cancelled_quorum",
            Self::CancelledCaller => "cancelled_caller",
            Self::ResponseTooLarge => "response_too_large",
            Self::Challenge => "challenge",
            Self::Unrecognized => "unrecognized",
            Self::RequestError => "request_error",
            Self::Unknown => "unknown",
        }
    }
    pub(crate) fn retryable(self) -> bool {
        self == Self::Deadline
    }
    /// Whether the attempt's candidate snapshot may be retained.
    pub fn retains_snapshot(self) -> bool {
        matches!(
            self,
            Self::Results
                | Self::Empty
                | Self::FilteredEmpty
                | Self::Deadline
                | Self::RateLimitedDeadline
                | Self::CancelledMinResults
                | Self::CancelledQuorum
        )
    }
    /// Whether this observation ended at a shared discovery deadline.
    pub fn is_deadline(self) -> bool {
        matches!(self, Self::Deadline | Self::RateLimitedDeadline)
    }
    /// Censored observations are lower bounds; failures have unknown timing status.
    pub fn timing_censored(self) -> Option<bool> {
        match self {
            Self::Results | Self::Empty | Self::FilteredEmpty => Some(false),
            Self::Deadline
            | Self::RateLimitedDeadline
            | Self::CancelledMinResults
            | Self::CancelledQuorum
            | Self::CancelledCaller => Some(true),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_unknown_reports_round_trip_without_schema_changes() {
        for value in [
            "results",
            "empty",
            "filtered_empty",
            "deadline",
            "rate_limited_deadline",
            "cancelled_min_results",
            "cancelled_quorum",
            "cancelled_caller",
            "response_too_large",
            "challenge",
            "unrecognized",
            "request_error",
            "future_outcome",
            "",
        ] {
            let input = serde_json::json!({"engine":"bing", "query":"q", "elapsed_ms":3, "result_count":0, "retries":0, "success":false, "outcome":value});
            let report: crate::ProviderSearchDiagnostic = serde_json::from_value(input).unwrap();
            assert_eq!(report.discovery_attempt, 1);
            assert_eq!(serde_json::to_value(&report).unwrap()["outcome"], value);
            let typed = ProviderOutcome::from_report(&report.outcome);
            if matches!(value, "future_outcome" | "") {
                assert_eq!(typed, ProviderOutcome::Unknown);
                assert!(!typed.retryable());
                assert!(!typed.retains_snapshot());
                assert_eq!(typed.timing_censored(), None);
            } else {
                assert_eq!(typed.as_str(), value);
            }
        }
    }
}
