//! Checked numeric boundaries shared by library request entry points.
use std::time::{Duration, Instant};

use crate::search::KestrelError;

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
