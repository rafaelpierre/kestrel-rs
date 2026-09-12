//! Provider-independent, bounded fanout events. Only adapters inspect wire formats.
use super::*;
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot};

mod records;

tokio::task_local! {
    pub(super) static PUBLISHER: Publisher;
}

#[derive(Clone)]
pub(super) struct Publisher {
    pub sender: mpsc::Sender<Batch>,
    pub index: usize,
    pub engine: Engine,
    pub query: String,
    pub query_syntax: QuerySyntax,
}

/// A cumulative snapshot of complete, normalized records from one provider.
/// Acknowledgement prevents reading more data before the collector checks its target.
pub(super) struct Batch {
    index: usize,
    results: Vec<SearchResult>,
    resume: oneshot::Sender<()>,
}

pub(super) async fn collect<F>(
    mut pending: FuturesUnordered<F>,
    quorum: Option<usize>,
    minimum: Option<usize>,
    signal: Option<Arc<AtomicU8>>,
    mut receiver: Option<mpsc::Receiver<Batch>>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    let mut completed = BTreeMap::new();
    let mut partial = BTreeMap::new();
    let mut cancelled = 0;
    while !pending.is_empty() {
        let resume = tokio::select! {
            outcome = pending.next() => {
                let Some((index, outcome)) = outcome else { break };
                // EOF replaces the snapshot. Deadlines retain complete records;
                // malformed/error responses retract them before the threshold.
                let snapshot = partial.remove(&index);
                let outcome = if matches!(outcome, Err(KestrelError::SearchDeadline)) {
                    snapshot.filter(|r: &Vec<SearchResult>| !r.is_empty()).map(Ok).unwrap_or(outcome)
                } else { outcome };
                completed.insert(index, outcome);
                None
            }
            event = async {
                match &mut receiver {
                    Some(receiver) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Some(batch) => {
                        partial.insert(batch.index, batch.results);
                        Some(batch.resume)
                    }
                    None => { receiver = None; continue; }
                }
            }
        };
        let buckets: Vec<_> = completed
            .values()
            .filter_map(|r| r.as_ref().ok())
            .chain(partial.values())
            .collect();
        // A result target takes precedence: provider diversity must never delay it.
        let reached = match minimum {
            Some(minimum) => {
                buckets
                    .iter()
                    .flat_map(|r| r.iter().map(result_key))
                    .collect::<HashSet<_>>()
                    .len()
                    >= minimum
            }
            None => quorum.is_some_and(|n| buckets.iter().filter(|r| !r.is_empty()).count() >= n),
        };
        if reached {
            cancelled = pending.len();
            if let Some(signal) = &signal {
                signal.store(
                    if minimum.is_some() {
                        FANOUT_MIN_RESULTS
                    } else {
                        FANOUT_QUORUM
                    },
                    Ordering::Relaxed,
                );
            }
            break;
        }
        if let Some(resume) = resume {
            let _ = resume.send(());
        }
    }
    // Drop all pending reads before returning. Pools belong to the shared clients.
    drop(pending);
    completed.extend(partial.into_iter().map(|(i, r)| (i, Ok(r))));
    (completed.into_values().collect(), cancelled)
}

pub(super) struct Incremental {
    publisher: Publisher,
    decoder: encoding_rs::Decoder,
    records: records::Records,
    previous: Vec<SearchResult>,
    plan: QueryPlan,
}

impl Incremental {
    pub fn for_body(body: &ProviderBody) -> Option<Self> {
        #[cfg(test)]
        if probe::BATCH_ONLY
            .try_with(|disabled| *disabled)
            .unwrap_or(false)
        {
            return None;
        }
        if !(200..300).contains(&body.status) {
            return None;
        }
        let publisher = PUBLISHER.try_with(Clone::clone).ok()?;
        let plan = QueryPlan::parse(&publisher.query, publisher.query_syntax).ok()?;
        Some(Self {
            records: records::Records::new(body.engine),
            publisher,
            decoder: body.encoding.new_decoder(),
            previous: Vec::new(),
            plan,
        })
    }

    pub async fn push(&mut self, bytes: &[u8]) -> Result<(), KestrelError> {
        let mut decoded = String::with_capacity(
            self.decoder
                .max_utf8_buffer_length(bytes.len())
                .unwrap_or(bytes.len().saturating_mul(3) + 16),
        );
        let (status, consumed, _) = self.decoder.decode_to_string(bytes, &mut decoded, false);
        debug_assert_eq!(status, encoding_rs::CoderResult::InputEmpty);
        debug_assert_eq!(consumed, bytes.len());
        record_phase(Phase::Parse);
        #[cfg(test)]
        let parse_started = Instant::now();
        let snapshot = self.records.push(&decoded).await;
        #[cfg(test)]
        probe::parse_time(self.publisher.engine, parse_started.elapsed().as_micros());
        record_phase(Phase::Body);
        if let Some(mut results) = snapshot? {
            let raw_count = results.len();
            normalize_provider_results(&mut results, &self.publisher.query, &self.plan);
            let _ = PROVIDER_DIAGNOSTIC.try_with(|(diagnostics, index)| {
                if let Ok(mut entries) = diagnostics.lock() {
                    let entry = &mut entries[*index];
                    entry.raw_result_count = raw_count;
                    entry.result_count = results.len();
                    entry.filtered_count = raw_count - results.len();
                }
            });
            let results = with_provenance(results, self.publisher.engine, &self.publisher.query);
            #[cfg(test)]
            probe::results(self.publisher.engine, &results);
            if results == self.previous {
                return Ok(());
            }
            self.previous = results.clone();
            let (resume, acknowledged) = oneshot::channel();
            if self
                .publisher
                .sender
                .send(Batch {
                    index: self.publisher.index,
                    results,
                    resume,
                })
                .await
                .is_ok()
            {
                let _ = acknowledged.await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod probe;
