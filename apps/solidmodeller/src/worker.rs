//! Does everything slow on one background thread so the UI never waits:
//! rebuilds, opening files, and saving (which exports STEP).
//!
//! Rebuild requests that pile up collapse into the newest one, and starting
//! a newer rebuild (or an open) cancels the running one through the kernel's
//! budget. A panic in the kernel or the tessellator is reported as an error
//! for that job; the thread carries on. The last good model stays on this
//! thread, so a save can cache STEP without the model crossing to the UI.

use crate::mesh::{self, SceneMesh};
use sm_document::PartDocument;
use sm_kernel::{Budget, Model};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

enum Job {
    Rebuild {
        generation: u64,
        doc: PartDocument,
        cancel: Arc<AtomicBool>,
    },
    Open(PathBuf),
    Save {
        path: PathBuf,
        doc: PartDocument,
        /// The rebuild generation `doc` corresponds to.
        generation: u64,
    },
}

pub enum Done {
    Rebuilt {
        generation: u64,
        result: Result<SceneMesh, String>,
    },
    Opened {
        path: PathBuf,
        result: Result<PartDocument, String>,
    },
    Saved {
        path: PathBuf,
        /// Ok(true) if cached STEP geometry was written with the document.
        result: Result<bool, String>,
    },
}

pub struct Worker {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    generation: u64,
    /// Generation of the last rebuild result handed out.
    received: u64,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    /// `wake` is called after each result is ready, from the worker thread.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, results) = channel();
        std::thread::Builder::new()
            .name("worker".into())
            .spawn(move || run(job_rx, done_tx, wake))
            .expect("spawn worker thread");
        Worker {
            jobs: Some(jobs),
            results,
            generation: 0,
            received: 0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn send(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            // Fails only if the thread is gone, which `run` never does early.
            let _ = jobs.send(job);
        }
    }

    /// Queues a rebuild of `doc`, cancelling any earlier one.
    pub fn rebuild(&mut self, doc: PartDocument) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self.send(Job::Rebuild {
            generation: self.generation,
            doc,
            cancel: self.cancel.clone(),
        });
    }

    /// Loads a file; the current rebuild is cancelled since its result is
    /// about to be replaced.
    pub fn open(&mut self, path: PathBuf) {
        self.cancel.store(true, Ordering::Relaxed);
        self.send(Job::Open(path));
    }

    /// Saves `doc`, which must be the document last passed to [`rebuild`].
    pub fn save(&self, path: PathBuf, doc: PartDocument) {
        self.send(Job::Save {
            path,
            doc,
            generation: self.generation,
        });
    }

    /// True until the result of the latest rebuild arrives.
    pub fn busy(&self) -> bool {
        self.received != self.generation
    }

    /// Finished jobs, oldest first. Results of superseded rebuilds are
    /// dropped.
    pub fn poll(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        while let Ok(done) = self.results.try_recv() {
            if let Done::Rebuilt { generation, .. } = done {
                if generation != self.generation {
                    continue;
                }
                self.received = generation;
            }
            out.push(done);
        }
        out
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Stop a long rebuild and let the thread exit when its queue closes.
        self.cancel.store(true, Ordering::Relaxed);
        self.jobs = None;
    }
}

fn run(jobs: Receiver<Job>, done: Sender<Done>, wake: impl Fn()) {
    // The last successful model and the generation it was built for.
    let mut last: Option<(u64, Model)> = None;
    while let Ok(first) = jobs.recv() {
        let mut batch = vec![first];
        batch.extend(jobs.try_iter());
        // Only the newest queued rebuild is worth running.
        let newest = batch.iter().rposition(|j| matches!(j, Job::Rebuild { .. }));
        for (i, job) in batch.into_iter().enumerate() {
            let result = match job {
                Job::Rebuild { .. } if Some(i) != newest => continue,
                Job::Rebuild {
                    generation,
                    doc,
                    cancel,
                } => {
                    let result = guarded(|| rebuild(&doc, &cancel));
                    let result = result.map(|(model, scene)| {
                        last = Some((generation, model));
                        scene
                    });
                    if result.is_err() {
                        last = None;
                    }
                    Done::Rebuilt { generation, result }
                }
                Job::Open(path) => Done::Opened {
                    result: guarded(|| {
                        sm_format::open(&path)
                            .map(|f| f.document)
                            .map_err(|e| e.to_string())
                    }),
                    path,
                },
                Job::Save {
                    path,
                    doc,
                    generation,
                } => {
                    let model = last
                        .as_ref()
                        .filter(|(g, _)| *g == generation)
                        .map(|(_, m)| m);
                    Done::Saved {
                        result: guarded(|| save(&path, &doc, model)),
                        path,
                    }
                }
            };
            if done.send(result).is_err() {
                return;
            }
            wake();
        }
    }
}

fn rebuild(doc: &PartDocument, cancel: &Arc<AtomicBool>) -> Result<(Model, SceneMesh), String> {
    let model = sm_kernel::rebuild_with(doc, &Budget::with_cancel(cancel.clone()))
        .map_err(|e| e.to_string())?;
    let scene = mesh::tessellate(&model, cancel).map_err(|e| e.to_string())?;
    Ok((model, scene))
}

fn save(path: &Path, doc: &PartDocument, model: Option<&Model>) -> Result<bool, String> {
    let name = path
        .file_name()
        .map_or_else(|| "part".into(), |n| n.to_string_lossy().into_owned());
    // Cache geometry only when it was built from exactly this document.
    let step = model.map(|m| sm_step::export(m, &name));
    sm_format::save(path, doc, step.as_deref()).map_err(|e| e.to_string())?;
    Ok(step.is_some())
}

/// Runs `f`, turning a panic into an error message.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".into());
        Err(format!("internal error: {msg}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use std::time::{Duration, Instant};

    fn wait(worker: &mut Worker, n: usize) -> Vec<Done> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut out = Vec::new();
        while out.len() < n {
            out.extend(worker.poll());
            assert!(Instant::now() < deadline, "worker timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    fn rebuilt(done: Done) -> (u64, Result<SceneMesh, String>) {
        match done {
            Done::Rebuilt { generation, result } => (generation, result),
            _ => panic!("expected a rebuild result"),
        }
    }

    #[test]
    fn only_the_latest_rebuild_is_reported() {
        let mut worker = Worker::new(|| {});
        let mut broken = demo::document();
        broken.features[0].suppressed = true;
        worker.rebuild(broken.clone());
        worker.rebuild(broken);
        worker.rebuild(demo::document());
        let (generation, result) = rebuilt(wait(&mut worker, 1).pop().unwrap());
        assert_eq!(generation, 3);
        assert!(!worker.busy());
        assert_eq!(result.unwrap().mesh.bodies.len(), 1);
    }

    #[test]
    fn failures_come_back_as_messages() {
        let mut worker = Worker::new(|| {});
        let mut doc = demo::document();
        // Suppressing a sketch its extrusion needs is an error, not a panic.
        doc.features[0].suppressed = true;
        worker.rebuild(doc);
        let (_, result) = rebuilt(wait(&mut worker, 1).pop().unwrap());
        assert!(!result.expect_err("rebuild should fail").is_empty());
    }

    #[test]
    fn panics_become_errors() {
        let r: Result<(), String> = guarded(|| panic!("boom"));
        assert_eq!(r, Err("internal error: boom".into()));
        let r: Result<(), String> = guarded(|| panic!("{}", String::from("formatted")));
        assert_eq!(r, Err("internal error: formatted".into()));
    }

    #[test]
    fn cancelled_rebuild_is_an_error() {
        let flag = Arc::new(AtomicBool::new(true));
        assert!(rebuild(&demo::document(), &flag).is_err());
    }

    #[test]
    fn empty_part_rebuilds_to_empty_scene() {
        let mut worker = Worker::new(|| {});
        worker.rebuild(PartDocument::new("Empty", sm_document::LengthUnit::Mm));
        let (_, result) = rebuilt(wait(&mut worker, 1).pop().unwrap());
        assert!(result.unwrap().mesh.indices.is_empty());
    }

    #[test]
    fn saves_with_cache_only_for_the_matching_rebuild_and_reopens() {
        let dir = std::env::temp_dir().join(format!("sm-worker-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.smp");
        let mut worker = Worker::new(|| {});
        let doc = demo::document();

        // Save queued right behind the rebuild uses its model.
        worker.rebuild(doc.clone());
        worker.save(path.clone(), doc.clone());
        let done = wait(&mut worker, 2);
        assert!(matches!(
            &done[1],
            Done::Saved {
                result: Ok(true),
                ..
            }
        ));

        // After a failed rebuild there is no matching model: no cache.
        let mut broken = doc.clone();
        broken.features[0].suppressed = true;
        worker.rebuild(broken.clone());
        worker.save(path.clone(), broken);
        let done = wait(&mut worker, 2);
        assert!(matches!(
            &done[1],
            Done::Saved {
                result: Ok(false),
                ..
            }
        ));

        worker.open(path.clone());
        match wait(&mut worker, 1).pop().unwrap() {
            Done::Opened { result, .. } => {
                assert!(result.unwrap().features[0].suppressed);
            }
            _ => panic!("expected an open result"),
        }
        worker.open(dir.join("missing.smp"));
        match wait(&mut worker, 1).pop().unwrap() {
            Done::Opened { result, .. } => assert!(result.is_err()),
            _ => panic!("expected an open result"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
