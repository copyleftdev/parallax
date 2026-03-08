//! Hybrid Logical Clock timestamps.
//!
//! **Spec reference:** `specs/01-data-model.md` §1.6

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hybrid Logical Clock timestamp.
///
/// Combines wall-clock milliseconds with a logical counter to provide
/// unique, monotonically increasing timestamps even when wall clocks
/// are close together.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Timestamp {
    /// Milliseconds since Unix epoch.
    pub wall_ms: u64,
    /// Logical counter, incremented when wall_ms doesn't advance.
    pub logical: u32,
}

impl Timestamp {
    /// Create a timestamp from the current system time.
    pub fn now() -> Self {
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_millis() as u64;
        Timestamp {
            wall_ms,
            logical: 0,
        }
    }

    /// Advance the timestamp. If wall clock has advanced, reset logical.
    /// Otherwise, increment logical.
    pub fn tick(&mut self) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_millis() as u64;

        if now_ms > self.wall_ms {
            self.wall_ms = now_ms;
            self.logical = 0;
        } else {
            self.logical += 1;
        }
    }

    /// Merge with a remote timestamp (for receiving events from other nodes).
    pub fn merge(&mut self, remote: &Timestamp) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_millis() as u64;

        if now_ms > self.wall_ms && now_ms > remote.wall_ms {
            self.wall_ms = now_ms;
            self.logical = 0;
        } else if self.wall_ms == remote.wall_ms {
            self.logical = self.logical.max(remote.logical) + 1;
        } else if remote.wall_ms > self.wall_ms {
            self.wall_ms = remote.wall_ms;
            self.logical = remote.logical + 1;
        } else {
            self.logical += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_nonzero() {
        let ts = Timestamp::now();
        assert!(ts.wall_ms > 0);
    }

    #[test]
    fn tick_advances() {
        let mut ts = Timestamp::now();
        let before = ts;
        ts.tick();
        assert!(ts > before);
    }

    #[test]
    fn default_is_epoch() {
        let ts = Timestamp::default();
        assert_eq!(ts.wall_ms, 0);
        assert_eq!(ts.logical, 0);
    }

    #[test]
    fn ordering_wall_then_logical() {
        let a = Timestamp {
            wall_ms: 100,
            logical: 5,
        };
        let b = Timestamp {
            wall_ms: 100,
            logical: 6,
        };
        let c = Timestamp {
            wall_ms: 101,
            logical: 0,
        };
        assert!(a < b);
        assert!(b < c);
    }
}
