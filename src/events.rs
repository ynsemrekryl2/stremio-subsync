//! Recent log lines kept in memory for the dashboard (SS-029). Every line
//! still goes to stderr; the last `CAPACITY` are also kept here. Lines are
//! the same log-safe text as stderr: counts and labels, never tokens, links,
//! ids, filenames or subtitle text.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CAPACITY: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub seq: u64,
    pub at_ms: u64,
    /// Short group name such as `k2` or `http`, for filtering.
    pub area: &'static str,
    pub line: String,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub started_ms: u64,
    /// Pass back as `after` to get only newer events.
    pub next: u64,
    /// Some events after the requested one already fell out of the buffer.
    pub missed: bool,
    pub events: Vec<Event>,
}

struct Log {
    started_ms: u64,
    next: u64,
    events: VecDeque<Event>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn log() -> &'static Mutex<Log> {
    static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
    LOG.get_or_init(|| {
        Mutex::new(Log {
            started_ms: now_ms(),
            next: 1,
            events: VecDeque::with_capacity(CAPACITY),
        })
    })
}

/// Prints `line` to stderr and keeps it for the dashboard.
pub fn record(area: &'static str, line: String) {
    eprintln!("{line}");
    let Ok(mut log) = log().lock() else { return };
    let seq = log.next;
    log.next += 1;
    if log.events.len() == CAPACITY {
        log.events.pop_front();
    }
    log.events.push_back(Event {
        seq,
        at_ms: now_ms(),
        area,
        line,
    });
}

/// Events with `seq > after`, oldest first.
pub fn since(after: u64) -> Page {
    let Ok(log) = log().lock() else {
        return Page {
            started_ms: 0,
            next: after,
            missed: false,
            events: Vec::new(),
        };
    };
    let events: Vec<Event> = log
        .events
        .iter()
        .filter(|e| e.seq > after)
        .cloned()
        .collect();
    let oldest = log.events.front().map_or(log.next, |e| e.seq);
    Page {
        started_ms: log.started_ms,
        next: log.next - 1,
        missed: after + 1 < oldest,
        events,
    }
}

/// `eprintln!` that also keeps the line for the dashboard.
macro_rules! event {
    ($area:literal, $($arg:tt)*) => {
        $crate::events::record($area, format!($($arg)*))
    };
}
pub(crate) use event;

#[cfg(test)]
mod tests {
    use super::*;

    /// The overflow test would push the other test's lines out.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn keeps_order_and_returns_only_newer_lines() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The buffer is shared with other tests, so look only at own lines.
        let before = since(0).next;
        event!("test", "first {}", 1);
        event!("test", "second");
        let page = since(before);
        let own: Vec<_> = page
            .events
            .iter()
            .filter(|e| e.area == "test")
            .map(|e| e.line.as_str())
            .collect();
        assert!(own.starts_with(&["first 1", "second"]), "{own:?}");
        assert!(page.next >= before + 2);
        assert!(since(page.next).events.iter().all(|e| e.seq > page.next));
    }

    #[test]
    fn old_lines_fall_out_and_are_reported_missed() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let before = since(0).next;
        for _ in 0..=CAPACITY {
            event!("test", "overflow");
        }
        let page = since(before);
        assert!(page.events.len() <= CAPACITY);
        assert!(page.missed);
    }
}
