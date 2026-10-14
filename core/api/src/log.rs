//! Debug log (section 19): an in-memory ring buffer of 500 lines, never
//! written to disk. Lines carry no IDs, keys, tags, nicknames or content;
//! peers appear as per-session counters (peer-1, peer-2, …).

use std::collections::{HashMap, VecDeque};

pub const LOG_LINES: usize = 500;

#[derive(Default)]
pub struct DebugLog {
    lines: VecDeque<String>,
    peers: HashMap<u64, u32>,
    next_peer: u32,
}

impl DebugLog {
    pub fn peer(&mut self, handle: u64) -> String {
        let n = *self.peers.entry(handle).or_insert_with(|| {
            self.next_peer += 1;
            self.next_peer
        });
        format!("peer-{n}")
    }

    pub fn push(&mut self, now_ms: u64, line: impl Into<String>) {
        let secs = now_ms / 1000;
        let ts = format!(
            "{:02}:{:02}:{:02}",
            (secs / 3600) % 24,
            (secs / 60) % 60,
            secs % 60
        );
        self.lines.push_back(format!("{ts} {}", line.into()));
        while self.lines.len() > LOG_LINES {
            self.lines.pop_front();
        }
    }

    pub fn text(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_and_peer_counters() {
        let mut l = DebugLog::default();
        assert_eq!(l.peer(77), "peer-1");
        assert_eq!(l.peer(99), "peer-2");
        assert_eq!(l.peer(77), "peer-1");
        for i in 0..600 {
            l.push(i * 1000, format!("line {i}"));
        }
        assert_eq!(l.len(), LOG_LINES);
        assert!(l.text().ends_with("line 599"));
        assert!(!l.text().contains("line 99\n"));
    }
}
