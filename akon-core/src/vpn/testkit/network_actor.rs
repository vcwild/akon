//! Network actor — controls health-check reachability over time.
//!
//! [`NetworkActor`] lets a scenario script connectivity (reachable /
//! unreachable / a per-poll sequence) so the reconnection logic can be
//! exercised **offline**, without real HTTP requests or affecting the host's
//! actual internet access.

/// Outcome of a single connectivity poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    /// The endpoint responded (health check would succeed).
    Up,
    /// The endpoint was unreachable (health check would fail).
    Down,
}

impl Reachability {
    /// Whether this poll represents a healthy link.
    pub fn is_up(&self) -> bool {
        matches!(self, Reachability::Up)
    }
}

/// In-memory controller of simulated connectivity.
#[derive(Debug, Clone)]
pub struct NetworkActor {
    /// Per-poll reachability. When the script is exhausted the final value
    /// repeats indefinitely (a steady state).
    script: Vec<Reachability>,
    cursor: usize,
}

impl NetworkActor {
    /// Always reachable.
    pub fn reachable() -> Self {
        Self {
            script: vec![Reachability::Up],
            cursor: 0,
        }
    }

    /// Always unreachable.
    pub fn unreachable() -> Self {
        Self {
            script: vec![Reachability::Down],
            cursor: 0,
        }
    }

    /// A scripted per-poll reachability sequence.
    ///
    /// Each `true` is a healthy poll, each `false` a failed one. After the last
    /// entry, the final value persists.
    pub fn script(per_poll: Vec<bool>) -> Self {
        let script: Vec<Reachability> = per_poll
            .into_iter()
            .map(|up| {
                if up {
                    Reachability::Up
                } else {
                    Reachability::Down
                }
            })
            .collect();
        Self {
            script: if script.is_empty() {
                vec![Reachability::Up]
            } else {
                script
            },
            cursor: 0,
        }
    }

    /// Convenience: healthy for `up` polls, then down for `down` polls, then
    /// healthy again forever (a recoverable interruption).
    pub fn interruption(up: usize, down: usize) -> Self {
        let mut per_poll = vec![true; up];
        per_poll.extend(std::iter::repeat(false).take(down));
        per_poll.push(true); // recovery steady-state
        Self::script(per_poll)
    }

    /// Poll the current reachability and advance the cursor.
    pub fn poll(&mut self) -> Reachability {
        let value = self
            .script
            .get(self.cursor)
            .copied()
            .unwrap_or_else(|| *self.script.last().expect("script is non-empty"));
        if self.cursor + 1 < self.script.len() {
            self.cursor += 1;
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachable_always_up() {
        let mut net = NetworkActor::reachable();
        for _ in 0..5 {
            assert!(net.poll().is_up());
        }
    }

    #[test]
    fn unreachable_always_down() {
        let mut net = NetworkActor::unreachable();
        for _ in 0..5 {
            assert!(!net.poll().is_up());
        }
    }

    #[test]
    fn script_then_steady_state() {
        let mut net = NetworkActor::script(vec![true, false, false]);
        assert!(net.poll().is_up());
        assert!(!net.poll().is_up());
        assert!(!net.poll().is_up());
        // exhausted -> last value (false) persists
        assert!(!net.poll().is_up());
    }

    #[test]
    fn interruption_recovers() {
        let mut net = NetworkActor::interruption(2, 2);
        assert!(net.poll().is_up()); // up
        assert!(net.poll().is_up()); // up
        assert!(!net.poll().is_up()); // down
        assert!(!net.poll().is_up()); // down
        assert!(net.poll().is_up()); // recovered
        assert!(net.poll().is_up()); // steady up
    }
}
