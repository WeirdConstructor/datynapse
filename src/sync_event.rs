use std::sync::{Arc, Mutex, Condvar};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct SyncEvent<T>(Arc<SyncEventImpl<T>>);

impl<T> Clone for SyncEvent<T> {
    fn clone(&self) -> Self {
        SyncEvent(self.0.clone())
    }
}

struct SyncEventImpl<T> {
    value:      Mutex<Option<T>>,
    available:  AtomicBool,
    condvar:    Condvar,
}

impl<T> SyncEvent<T> {
    pub fn new() -> Self {
        Self (Arc::new(SyncEventImpl {
            value:      Mutex::new(None),
            available:  AtomicBool::new(false),
            condvar:    Condvar::new(),
        }))
    }

    pub fn send(&self, val: T) {
        *self.0.value.lock().expect("Mutex not poisoned") = Some(val);
        self.0.available.store(true, Ordering::Relaxed);
        self.0.condvar.notify_one()
    }

    pub fn is_available(&self) -> bool {
        self.0.available.load(Ordering::Relaxed)
    }

    pub fn recv_timeout(&self, dur: Duration) -> Option<T> {
        let mut result =
            self.0.condvar.wait_timeout_while(
                self.0.value.lock().expect("Mutex not poisoned"),
                dur,
                |value| value.as_ref().is_none())
            .expect("Mutex in condition not poisoned");

        if result.1.timed_out() {
            return None;
        }

        self.0.available.store(false, Ordering::Relaxed);
        result.0.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_event_works() {
        let mut ev = SyncEvent::new();
        let mut evt = ev.clone();

        assert_eq!(ev.recv_timeout(Duration::from_millis(100)), None);

        std::thread::spawn(move || { evt.send(true); });

        assert_eq!(
            ev.recv_timeout(Duration::from_millis(1000)),
            Some(true));
    }
}
