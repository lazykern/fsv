//! Background jobs: threads in the app, a queue drained by `settle()` in
//! tests. A worker never takes the UI down: a panic comes back as `Err`.
//! Staleness is the caller's business (generation and query checks).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};

type Job<T> = Box<dyn FnOnce() -> T + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkersMode {
    Threads,
    Inline,
}

pub struct Workers<M, T> {
    mode: WorkersMode,
    tx: Sender<(M, Result<T, String>)>,
    rx: Receiver<(M, Result<T, String>)>,
    queue: Vec<(M, Job<T>)>,
    running: usize,
}

fn run<T>(job: Job<T>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(job)).map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "worker failed".into())
    })
}

impl<M: Send + 'static, T: Send + 'static> Workers<M, T> {
    pub fn new(mode: WorkersMode) -> Self {
        let (tx, rx) = channel();
        Workers {
            mode,
            tx,
            rx,
            queue: vec![],
            running: 0,
        }
    }

    pub fn spawn(&mut self, meta: M, job: impl FnOnce() -> T + Send + 'static) {
        match self.mode {
            WorkersMode::Inline => self.queue.push((meta, Box::new(job))),
            WorkersMode::Threads => {
                let tx = self.tx.clone();
                self.running += 1;
                std::thread::spawn(move || {
                    let _ = tx.send((meta, run(Box::new(job))));
                });
            }
        }
    }

    /// Finished jobs (inline mode runs the queued ones now).
    pub fn drain(&mut self) -> Vec<(M, Result<T, String>)> {
        for (meta, job) in std::mem::take(&mut self.queue) {
            let _ = self.tx.send((meta, run(job)));
        }
        let out: Vec<_> = self.rx.try_iter().collect();
        if self.mode == WorkersMode::Threads {
            self.running = self.running.saturating_sub(out.len());
        }
        out
    }

    /// Jobs queued or still running.
    pub fn pending(&self) -> usize {
        self.queue.len() + self.running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_jobs_run_on_drain_and_panics_are_errors() {
        let mut w: Workers<u8, i32> = Workers::new(WorkersMode::Inline);
        w.spawn(1, || 41 + 1);
        w.spawn(2, || panic!("boom"));
        assert_eq!(w.pending(), 2);
        let mut got = w.drain();
        got.sort_by_key(|(m, _)| *m);
        assert_eq!(got[0], (1, Ok(42)));
        assert_eq!(got[1], (2, Err("boom".into())));
        assert_eq!(w.pending(), 0);
    }
}
