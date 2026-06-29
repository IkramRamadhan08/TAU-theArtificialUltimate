use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

const THRESHOLD: usize = 5;
const COOLDOWN: Duration = Duration::from_secs(60);

pub struct CircuitBreaker {
    consecutive_errors: AtomicUsize,
    tripped_at: AtomicU64,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            consecutive_errors: AtomicUsize::new(0),
            tripped_at: AtomicU64::new(0),
        }
    }

    pub fn is_open(&self) -> bool {
        let tripped = self.tripped_at.load(Ordering::Relaxed);
        if tripped == 0 {
            return false;
        }
        let elapsed = Self::now_millis().saturating_sub(tripped);
        if elapsed >= COOLDOWN.as_millis() as u64 {
            self.tripped_at.store(0, Ordering::Relaxed);
            return false;
        }
        self.consecutive_errors.load(Ordering::Relaxed) >= THRESHOLD
    }

    pub fn record_success(&self) {
        self.consecutive_errors.store(0, Ordering::Relaxed);
        self.tripped_at.store(0, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        let count = self.consecutive_errors.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= THRESHOLD {
            let now = Self::now_millis();
            self.tripped_at.store(now, Ordering::Relaxed);
        }
    }

    pub fn retry_after(&self) -> Duration {
        let tripped = self.tripped_at.load(Ordering::Relaxed);
        if tripped == 0 {
            return Duration::ZERO;
        }
        let elapsed = Self::now_millis().saturating_sub(tripped);
        COOLDOWN.saturating_sub(Duration::from_millis(elapsed))
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
