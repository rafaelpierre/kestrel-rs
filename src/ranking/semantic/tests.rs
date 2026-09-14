use super::*;
use crate::model::{Engine, SourceOccurrence};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn candidate(url: &str, query: &str) -> SearchResult {
    SearchResult {
        title: url.into(),
        url: url.into(),
        display_url: url.into(),
        snippet: String::new(),
        content: None,
        bm25_score: Some(9.0),
        engine: None,
        query: Some(query.into()),
        engine_rank: None,
        sources: vec![SourceOccurrence {
            engine: Engine::Bing,
            query: query.into(),
            rank: 1,
        }],
    }
}
fn row(
    query: usize,
    candidate: usize,
    lexical: Option<f64>,
    semantic: Option<f64>,
) -> RelevanceEvidence {
    RelevanceEvidence {
        query,
        candidate,
        lexical,
        semantic,
    }
}
#[test]
fn hand_calculated_fusion_keeps_single_component_evidence() {
    let result = combine(
        vec![
            candidate("https://x/a", "q"),
            candidate("https://x/b", "q"),
            candidate("https://x/c", "q"),
        ],
        1,
        &[
            row(0, 0, Some(10.0), None),
            row(0, 1, None, Some(0.8)),
            row(0, 2, Some(5.0), Some(0.7)),
        ],
    )
    .unwrap();
    assert_eq!(
        result
            .results
            .iter()
            .map(|r| r.url.as_str())
            .collect::<Vec<_>>(),
        ["https://x/c", "https://x/a", "https://x/b"]
    );
    assert_eq!(result.ranks[0].score, 2.0 / 62.0);
    assert_eq!(result.ranks[1].score, 1.0 / 61.0);
    assert!(result.results.iter().all(|r| r.bm25_score == Some(9.0)));
}
#[test]
fn duplicates_queries_invalid_scores_and_order_are_explicit() {
    let input = vec![
        candidate("https://x/a#one", "q1"),
        candidate("https://x/a#two", "q2"),
        candidate("https://x/b", "q2"),
        candidate("https://x/c", "q"),
    ];
    let evidence = vec![
        row(0, 0, Some(f64::NAN), Some(1.0)),
        row(0, 1, Some(f64::INFINITY), Some(2.0)),
        row(1, 1, None, Some(-1.0)),
        row(1, 2, Some(0.0), Some(1.0)),
    ];
    let result = combine(input.clone(), 2, &evidence).unwrap();
    assert_eq!(result.results.len(), 3);
    assert_eq!(result.results[0].sources.len(), 2);
    assert_eq!(
        result.ranks.iter().find(|r| r.query == 0).unwrap().score,
        1.0 / 61.0
    );
    let mut reversed = evidence;
    reversed.reverse();
    let again = combine(input, 2, &reversed).unwrap();
    assert_eq!(result.results, again.results);
    assert_eq!(result.ranks.iter().filter(|r| r.query == 1).count(), 2);
}
#[test]
fn ties_use_url_and_boundaries_preserve_candidates() {
    let input = vec![
        candidate("https://x/b", "same"),
        candidate("https://x/a", "same"),
    ];
    let result = combine(
        input.clone(),
        2,
        &[
            row(0, 0, Some(1.0), None),
            row(0, 1, Some(1.0), None),
            row(1, 0, None, None),
        ],
    )
    .unwrap();
    assert_eq!(result.results[0].url, "https://x/a");
    assert_eq!(result.ranks[0].lexical_rank, Some(1));
    assert!(combine(input.clone(), 0, &[row(0, 0, None, None)]).is_err());
    assert!(combine(input.clone(), 1, &[row(0, 2, None, None)]).is_err());
    assert_eq!(combine(input, 0, &[]).unwrap().results.len(), 2);
    assert!(combine(vec![], 0, &[]).unwrap().results.is_empty());
}
fn limits() -> ScoringLimits {
    ScoringLimits {
        max_pairs: 2,
        max_query_bytes: 4,
        max_text_bytes: 4,
        max_total_bytes: 8,
        timeout: Duration::from_millis(10),
    }
}
fn inputs() -> Vec<ScoringInput<'static>> {
    vec![
        ScoringInput {
            id: 3,
            query: "q",
            text: "a",
        },
        ScoringInput {
            id: 8,
            query: "q",
            text: "b",
        },
    ]
}
struct Mock {
    output: Vec<ScoringOutput>,
    error: Option<ScoringError>,
    pending: bool,
    dropped: Arc<AtomicBool>,
}
struct Guard(Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl SemanticScorer for Mock {
    fn identity(&self) -> ScorerIdentity {
        ScorerIdentity {
            backend: "fixture".into(),
            model_revision: "v1".into(),
            configuration: "page".into(),
        }
    }
    async fn availability(&self) -> Result<(), ScoringError> {
        self.error.clone().map_or(Ok(()), Err)
    }
    async fn score(&self, _: &[ScoringInput<'_>]) -> Result<Vec<ScoringOutput>, ScoringError> {
        let _guard = Guard(self.dropped.clone());
        if self.pending {
            std::future::pending::<()>().await;
        }
        Ok(self.output.clone())
    }
}
fn mock() -> Mock {
    Mock {
        output: vec![
            ScoringOutput { id: 8, score: None },
            ScoringOutput {
                id: 3,
                score: Some(-0.2),
            },
        ],
        error: None,
        pending: false,
        dropped: Arc::default(),
    }
}
#[tokio::test]
async fn batch_mapping_errors_and_bounds() {
    let mut backend = mock();
    let output = score_batch(&backend, &inputs(), limits()).await.unwrap();
    assert_eq!(output[0].id, 3);
    assert_eq!(output[0].score, Some(-0.2));
    assert_eq!(output[1].score, None);
    for output in [
        vec![],
        vec![ScoringOutput {
            id: 99,
            score: None,
        }],
        vec![
            ScoringOutput { id: 3, score: None },
            ScoringOutput { id: 3, score: None },
        ],
        vec![
            ScoringOutput {
                id: 3,
                score: Some(f64::NAN),
            },
            ScoringOutput { id: 8, score: None },
        ],
    ] {
        backend.output = output;
        assert_eq!(
            score_batch(&backend, &inputs(), limits())
                .await
                .unwrap_err(),
            ScoringError::InvalidOutput
        );
    }
    for error in [
        ScoringError::Unavailable("model".into()),
        ScoringError::Backend("failed".into()),
    ] {
        backend.error = Some(error.clone());
        assert_eq!(
            score_batch(&backend, &inputs(), limits())
                .await
                .unwrap_err(),
            error
        );
    }
    let mut invalid = inputs();
    invalid[1].id = 3;
    assert_eq!(
        score_batch(&backend, &invalid, limits()).await.unwrap_err(),
        ScoringError::InvalidInput
    );
    for limit in [
        ScoringLimits {
            max_pairs: 1,
            ..limits()
        },
        ScoringLimits {
            max_query_bytes: 0,
            ..limits()
        },
        ScoringLimits {
            max_text_bytes: 0,
            ..limits()
        },
        ScoringLimits {
            max_total_bytes: 3,
            ..limits()
        },
        ScoringLimits {
            timeout: Duration::ZERO,
            ..limits()
        },
    ] {
        assert_eq!(
            score_batch(&backend, &inputs(), limit).await.unwrap_err(),
            ScoringError::InvalidInput
        );
    }
    assert!(
        score_batch(&backend, &[], limits())
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn timeout_and_caller_cancellation_drop_inference() {
    let mut backend = mock();
    backend.pending = true;
    assert_eq!(
        score_batch(&backend, &inputs(), limits())
            .await
            .unwrap_err(),
        ScoringError::Timeout
    );
    assert!(backend.dropped.load(Ordering::SeqCst));
    backend.dropped.store(false, Ordering::SeqCst);
    let input = inputs();
    let mut future = Box::pin(score_batch(
        &backend,
        &input,
        ScoringLimits {
            timeout: Duration::from_secs(10),
            ..limits()
        },
    ));
    assert!(futures_util::poll!(&mut future).is_pending());
    drop(future);
    assert!(backend.dropped.load(Ordering::SeqCst));
}
#[test]
fn cache_identity_covers_inputs_and_revision_but_not_transient_ids() {
    let mut identity = mock().identity();
    let mut input = inputs().remove(0);
    let original = cache_key(&identity, &input);
    input.id = 99;
    assert_eq!(original, cache_key(&identity, &input));
    input.text = "b";
    assert_ne!(original, cache_key(&identity, &input));
    input.text = "a";
    input.query = "z";
    assert_ne!(original, cache_key(&identity, &input));
    input.query = "q";
    identity.model_revision = "v2".into();
    assert_ne!(original, cache_key(&identity, &input));
    identity = mock().identity();
    identity.configuration = "passage".into();
    assert_ne!(original, cache_key(&identity, &input));
    identity = mock().identity();
    identity.backend = "other".into();
    assert_ne!(original, cache_key(&identity, &input));
}

#[test]
fn invalid_urls_are_rejected_instead_of_collapsed() {
    assert!(combine(vec![candidate("invalid", "q")], 1, &[]).is_err());
}
