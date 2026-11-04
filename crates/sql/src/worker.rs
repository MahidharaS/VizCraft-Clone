//! Runs SQL on a background thread so the UI stays responsive; long queries can be cancelled.

use std::sync::mpsc::{Receiver, Sender, channel};

use crate::{Engine, Result, StatementOutput};

#[derive(Clone, Debug)]
pub enum Job {
    /// Run every statement of a script; `cap` rows per result.
    Script { sql: String, cap: usize },
    /// The plan of one statement.
    Explain { sql: String, analyze: bool },
    /// The full result of a query written to a file (`COPY … TO`).
    Export { sql: String, path: String },
}

#[derive(Clone, Debug)]
pub struct JobDone {
    pub id: u64,
    pub job: Job,
    pub outputs: Vec<StatementOutput>,
    pub elapsed_ms: f64,
}

pub struct Worker {
    tx: Sender<(u64, Job)>,
    rx: Receiver<JobDone>,
    #[cfg(feature = "duckdb")]
    interrupt: std::sync::Arc<duckdb::InterruptHandle>,
    next: u64,
    /// Jobs submitted and not finished yet.
    pub pending: Vec<u64>,
}

impl Worker {
    #[cfg(feature = "duckdb")]
    pub fn new(engine: &Engine) -> Result<Worker> {
        let conn = engine.connection()?;
        let interrupt = conn.interrupt_handle();
        let (tx, jobs) = channel::<(u64, Job)>();
        let (done, rx) = channel::<JobDone>();
        std::thread::Builder::new()
            .name("vizcraft-sql".into())
            .spawn(move || {
                while let Ok((id, job)) = jobs.recv() {
                    let t0 = std::time::Instant::now();
                    let outputs = match &job {
                        Job::Script { sql, cap } => crate::duck::run_script(&conn, sql, *cap),
                        Job::Explain { sql, analyze } => match crate::duck::explain(&conn, sql, *analyze) {
                            Ok(text) => vec![StatementOutput::Plan { sql: sql.clone(), text, elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 }],
                            Err(message) => vec![StatementOutput::Error { sql: sql.clone(), message }],
                        },
                        Job::Export { sql, path } => match crate::duck::export(&conn, sql, path) {
                            Ok(()) => vec![],
                            Err(message) => vec![StatementOutput::Error { sql: sql.clone(), message }],
                        },
                    };
                    if done.send(JobDone { id, job, outputs, elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Worker { tx, rx, interrupt, next: 1, pending: vec![] })
    }

    #[cfg(not(feature = "duckdb"))]
    pub fn new(engine: &Engine) -> Result<Worker> {
        let _ = engine;
        let _: (Sender<(u64, Job)>, Receiver<(u64, Job)>) = channel();
        Err(crate::stub::UNAVAILABLE.into())
    }

    /// Queues a job; returns its id.
    pub fn submit(&mut self, job: Job) -> u64 {
        let id = self.next;
        self.next += 1;
        if self.tx.send((id, job)).is_ok() {
            self.pending.push(id);
        }
        id
    }

    /// Finished jobs since the last call.
    pub fn poll(&mut self) -> Vec<JobDone> {
        let mut out = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            self.pending.retain(|p| *p != d.id);
            out.push(d);
        }
        out
    }

    /// Waits for a job (tests, scripted runs).
    pub fn wait(&mut self, timeout: std::time::Duration) -> Option<JobDone> {
        let d = self.rx.recv_timeout(timeout).ok()?;
        self.pending.retain(|p| *p != d.id);
        Some(d)
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Interrupts the running query.
    pub fn cancel(&self) {
        #[cfg(feature = "duckdb")]
        self.interrupt.interrupt();
    }
}
