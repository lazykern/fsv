//! One-shot timers on a virtual clock (`set_timer` / `.stop()`): the run
//! loop feeds real elapsed time, tests advance it by hand.

use std::time::Duration;

#[derive(Debug)]
pub struct Timers<T> {
    entries: Vec<(T, Duration)>,
}

impl<T> Default for Timers<T> {
    fn default() -> Self {
        Timers { entries: vec![] }
    }
}

impl<T: PartialEq + Clone> Timers<T> {
    /// (Re)start `id` to fire `delay` after `now`.
    pub fn set(&mut self, id: T, now: Duration, delay: Duration) {
        self.cancel(&id);
        self.entries.push((id, now + delay));
    }

    pub fn cancel(&mut self, id: &T) {
        self.entries.retain(|(t, _)| t != id);
    }

    pub fn is_set(&self, id: &T) -> bool {
        self.entries.iter().any(|(t, _)| t == id)
    }

    /// Timers due at `now`, earliest first, removed from the set.
    pub fn due(&mut self, now: Duration) -> Vec<T> {
        let mut due: Vec<(T, Duration)> = vec![];
        self.entries.retain(|(t, at)| {
            if *at <= now {
                due.push((t.clone(), *at));
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(_, at)| *at);
        due.into_iter().map(|(t, _)| t).collect()
    }

    pub fn next_deadline(&self) -> Option<Duration> {
        self.entries.iter().map(|(_, at)| *at).min()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_replaces_and_due_fires_in_order() {
        let ms = Duration::from_millis;
        let mut t = Timers::default();
        t.set("a", ms(0), ms(300));
        t.set("b", ms(0), ms(100));
        t.set("a", ms(200), ms(300));
        assert_eq!(t.next_deadline(), Some(ms(100)));
        assert_eq!(t.due(ms(300)), vec!["b"]);
        assert_eq!(t.due(ms(500)), vec!["a"]);
        assert!(t.is_empty());
    }
}
