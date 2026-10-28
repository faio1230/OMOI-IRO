//! Latest-value mailbox (capacity 1, docs/02 "実行とキュー"): the producer
//! never blocks; a value not yet taken is overwritten and counted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

pub struct Mailbox<T> {
    slot: Mutex<(Option<T>, bool)>,
    cv: Condvar,
    posted: AtomicU64,
    overwritten: AtomicU64,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Mailbox { slot: Mutex::new((None, false)), cv: Condvar::new(), posted: AtomicU64::new(0), overwritten: AtomicU64::new(0) }
    }
}

impl<T> Mailbox<T> {
    pub fn new() -> Mailbox<T> {
        Mailbox::default()
    }

    /// Store `v`, replacing an untaken value. Returns true if one was replaced.
    pub fn put(&self, v: T) -> bool {
        let mut g = self.slot.lock().unwrap();
        let replaced = g.0.replace(v).is_some();
        drop(g);
        self.posted.fetch_add(1, Ordering::Relaxed);
        if replaced {
            self.overwritten.fetch_add(1, Ordering::Relaxed);
        }
        self.cv.notify_one();
        replaced
    }

    pub fn try_take(&self) -> Option<T> {
        self.slot.lock().unwrap().0.take()
    }

    /// Wait up to `timeout` for a value. `None` on timeout or after `close`.
    pub fn take_timeout(&self, timeout: Duration) -> Option<T> {
        let g = self.slot.lock().unwrap();
        let (mut g, _) = self.cv.wait_timeout_while(g, timeout, |(v, closed)| v.is_none() && !*closed).unwrap();
        g.0.take()
    }

    /// Wake waiters; later `take_timeout` calls return at once when empty.
    pub fn close(&self) {
        self.slot.lock().unwrap().1 = true;
        self.cv.notify_all();
    }

    pub fn posted(&self) -> u64 {
        self.posted.load(Ordering::Relaxed)
    }

    pub fn overwritten(&self) -> u64 {
        self.overwritten.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn keeps_only_the_latest_and_counts_overwrites() {
        let m = Mailbox::new();
        assert!(!m.put(1));
        assert!(m.put(2));
        assert!(m.put(3));
        assert_eq!(m.try_take(), Some(3));
        assert_eq!(m.try_take(), None);
        assert_eq!((m.posted(), m.overwritten()), (3, 2));
    }

    #[test]
    fn take_waits_and_close_wakes() {
        let m = Arc::new(Mailbox::new());
        let p = m.clone();
        let h = std::thread::spawn(move || p.put(7));
        assert_eq!(m.take_timeout(Duration::from_secs(5)), Some(7));
        h.join().unwrap();
        m.close();
        assert_eq!(m.take_timeout(Duration::from_secs(5)), None);
    }
}
