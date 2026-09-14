//! Checked numeric boundaries shared by library request entry points.
use std::time::{Duration, Instant};

use crate::error::KestrelError;

pub(crate) fn duration(name: &str, value: Duration) -> Result<(), KestrelError> {
    if value.is_zero() || Instant::now().checked_add(value).is_none() {
        return Err(KestrelError::InvalidRequest(format!(
            "{name} must be nonzero and fit a monotonic clock deadline"
        )));
    }
    Ok(())
}

pub(crate) fn concurrency(name: &str, value: usize) -> Result<(), KestrelError> {
    if !(1..=tokio::sync::Semaphore::MAX_PERMITS).contains(&value) {
        return Err(KestrelError::InvalidRequest(format!(
            "{name} must be between 1 and {}",
            tokio::sync::Semaphore::MAX_PERMITS
        )));
    }
    Ok(())
}

pub(crate) fn deadline(name: &str, value: Duration) -> Result<tokio::time::Instant, KestrelError> {
    duration(name, value)?;
    tokio::time::Instant::now()
        .checked_add(value)
        .ok_or_else(|| {
            KestrelError::InvalidRequest(format!("{name} exceeds the monotonic clock range"))
        })
}

pub(crate) async fn before_deadline<T>(
    deadline: Option<tokio::time::Instant>,
    work: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    match deadline {
        Some(end) if tokio::time::Instant::now() >= end => Err(()),
        Some(end) => tokio::time::timeout_at(end, work).await.map_err(|_| ()),
        None => Ok(work.await),
    }
}
