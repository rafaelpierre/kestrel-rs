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
}

/// A cumulative snapshot of complete, normalized records from one provider.
/// Acknowledgement prevents reading more data before the collector checks its target.
pub(super) struct Batch {
    index: usize,
    results: Vec<SearchResult>,
    resume: oneshot::Sender<()>,
}

#[cfg(test)]
pub(super) async fn collect<F>(
    pending: FuturesUnordered<F>,
    minimum: usize,
    signal: Option<Arc<AtomicU8>>,
    receiver: Option<mpsc::Receiver<Batch>>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    collect_recording(pending, minimum, signal, receiver, None, None).await
}

#[cfg(test)]
pub(super) async fn collect_recording<F>(
    pending: FuturesUnordered<F>,
    minimum: usize,
    signal: Option<Arc<AtomicU8>>,
    receiver: Option<mpsc::Receiver<Batch>>,
    progress: Option<(
        crate::recovery::ProgressQueue,
        Vec<crate::recovery::UnitKey>,
    )>,
    deadline: Option<tokio::time::Instant>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    collect_replaying(
        pending,
        minimum,
        signal,
        receiver,
        progress,
        deadline,
        BTreeMap::new(),
    )
    .await
}

pub(super) async fn collect_replaying<F>(
    mut pending: FuturesUnordered<F>,
    minimum: usize,
    signal: Option<Arc<AtomicU8>>,
    mut receiver: Option<mpsc::Receiver<Batch>>,
    progress: Option<(
        crate::recovery::ProgressQueue,
        Vec<crate::recovery::UnitKey>,
    )>,
    deadline: Option<tokio::time::Instant>,
    recovered: BTreeMap<usize, crate::recovery::Snapshot>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    let mut sequence = 0u64;
    let mut completed = BTreeMap::new();
    let mut partial = BTreeMap::new();
    for (index, snapshot) in recovered {
        if snapshot.state == crate::recovery::State::Complete {
            completed.insert(index, Ok(snapshot.records));
        } else {
            partial.insert(index, snapshot.records);
        }
    }
    let mut cancelled = 0;
    while !pending.is_empty() {
        let resume = tokio::select! {
            () = async { match &progress { Some((queue,_)) => queue.store.cancelled().await, None => std::future::pending().await } } => {
                cancelled = pending.len();
                break;
            }

            outcome = pending.next() => {
                let Some((index, outcome)) = outcome else { break };
                // EOF replaces the snapshot. Deadlines retain complete records;
                // malformed/error responses retract them before the threshold.
                if let Some((queue, keys)) = &progress {
                    // One sequence across this query's discovery retries, within
                    // the invocation's shared bounded persistence writer.
                    sequence = DISCOVERY_SEQUENCE.try_with(|counter| counter.fetch_add(1, Ordering::Relaxed) + 1)
                        .unwrap_or_else(|_| sequence.saturating_add(1));
                    match &outcome {
                        Ok(records) => queue.enqueue(keys[index].clone(), sequence, crate::recovery::State::Complete, records, deadline).await,
                        Err(KestrelError::SearchDeadline) => (),
                        Err(_) => queue.enqueue(keys[index].clone(), sequence, crate::recovery::State::Invalid, &[], deadline).await,
                    }
                }
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
                        if let Some((queue, keys)) = &progress {
                            // One sequence across this query's discovery retries, within
                    // the invocation's shared bounded persistence writer.
                    sequence = DISCOVERY_SEQUENCE.try_with(|counter| counter.fetch_add(1, Ordering::Relaxed) + 1)
                        .unwrap_or_else(|_| sequence.saturating_add(1));
                            queue.enqueue(keys[batch.index].clone(), sequence, crate::recovery::State::Incomplete, &batch.results, deadline).await;
                        }
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
        // Only unique result count controls stopping; provider diversity is irrelevant.
        let reached = buckets
            .iter()
            .flat_map(|r| r.iter().map(result_key))
            .collect::<HashSet<_>>()
            .len()
            >= minimum;
        #[cfg(test)]
        let reached = probe::validation::observe_collector(&buckets, reached);
        if reached {
            cancelled = pending.len();
            if let Some(signal) = &signal {
                signal.store(FANOUT_MIN_RESULTS, Ordering::Relaxed);
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
        Some(Self {
            records: records::Records::new(body.engine),
            publisher,
            decoder: body.encoding.new_decoder(),
            previous: Vec::new(),
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
            crate::telemetry::results("stream.raw_snapshot", &results);
            let raw_count = results.len();
            normalize_provider_results(&mut results, &self.publisher.query);
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
            crate::telemetry::results("stream.accepted_snapshot", &results);
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
