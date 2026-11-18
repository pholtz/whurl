//! Background runs. Each job is a thread that owns the hurl child, polls it, and kills it when
//! its cancel flag is set. Results come back over a channel tagged with the job id, so a late
//! message from a cancelled job can't clobber a newer run of the same request.

use super::app::Key;
use crate::run::{self, Run, RunOpts};
use crate::workspace::{RequestFile, Workspace};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(5);

pub enum Outcome {
    Done(Run),
    Failed(String),
    Cancelled,
}

pub struct Msg {
    pub id: u64,
    pub key: Key,
    pub outcome: Outcome,
}

pub struct Job {
    pub id: u64,
    pub started: Instant,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Job {
    pub fn start(
        id: u64,
        key: Key,
        ws: &Workspace,
        req: RequestFile,
        opts: RunOpts,
        tx: Sender<Msg>,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let root = ws.root.clone();
        let handle = std::thread::spawn(move || {
            let ws = Workspace { root };
            let outcome = work(&ws, &req, &opts, &flag);
            let _ = tx.send(Msg { id, key, outcome });
        });
        Job {
            id,
            started: Instant::now(),
            cancel,
            handle: Some(handle),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Cancel and wait for the child to be killed (used on quit so no hurl is orphaned).
    pub fn shutdown(mut self) {
        self.cancel();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn work(ws: &Workspace, req: &RequestFile, opts: &RunOpts, cancel: &AtomicBool) -> Outcome {
    let mut p = match run::spawn(ws, req, opts) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(format!("{e:#}")),
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = p.child.kill();
            let _ = p.child.wait();
            return Outcome::Cancelled;
        }
        match p.child.try_wait() {
            Ok(Some(status)) => {
                return match p.finish(ws, status) {
                    Ok(r) => Outcome::Done(r),
                    Err(e) => Outcome::Failed(format!("{e:#}")),
                };
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => return Outcome::Failed(format!("waiting for hurl: {e}")),
        }
    }
}
