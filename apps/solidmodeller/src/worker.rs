//! Rebuilds the model on one background thread so the UI never waits on
//! truck's booleans. Requests that pile up while a rebuild runs collapse into
//! the newest one, so rapid edits cost one rebuild, not one each.

use crate::mesh::{self, SceneMesh};
use sm_document::PartDocument;
use sm_kernel::Model;
use std::sync::mpsc::{channel, Receiver, Sender};

pub struct Rebuilt {
    pub generation: u64,
    pub result: Result<(Model, SceneMesh), String>,
}

pub struct RebuildWorker {
    jobs: Sender<(u64, PartDocument)>,
    results: Receiver<Rebuilt>,
    generation: u64,
}

impl RebuildWorker {
    /// `wake` is called after each result is ready, from the worker thread.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, job_rx) = channel::<(u64, PartDocument)>();
        let (result_tx, results) = channel();
        std::thread::Builder::new()
            .name("rebuild".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Skip to the newest request.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let (generation, doc) = job;
                    let result = sm_kernel::rebuild(&doc)
                        .map(|model| {
                            let scene = mesh::tessellate(&model);
                            (model, scene)
                        })
                        .map_err(|e| e.to_string());
                    if result_tx.send(Rebuilt { generation, result }).is_err() {
                        return;
                    }
                    wake();
                }
            })
            .expect("spawn rebuild thread");
        RebuildWorker {
            jobs,
            results,
            generation: 0,
        }
    }

    /// Queues a rebuild of `doc`, superseding any earlier request.
    pub fn submit(&mut self, doc: PartDocument) {
        self.generation += 1;
        // The worker only exits once this sender is dropped.
        let _ = self.jobs.send((self.generation, doc));
    }

    /// True until the result of the latest [`submit`](Self::submit) arrives.
    pub fn busy(&self, last_received: u64) -> bool {
        last_received != self.generation
    }

    /// The result for the latest request, if it is ready. Results for
    /// superseded requests are discarded.
    pub fn poll(&self) -> Option<Rebuilt> {
        let mut latest = None;
        while let Ok(done) = self.results.try_recv() {
            if done.generation == self.generation {
                latest = Some(done);
            }
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use std::time::{Duration, Instant};

    fn wait(worker: &RebuildWorker) -> Rebuilt {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(r) = worker.poll() {
                return r;
            }
            assert!(Instant::now() < deadline, "rebuild timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn only_the_latest_request_is_reported() {
        let mut worker = RebuildWorker::new(|| {});
        let mut broken = demo::document();
        broken.features.clear();
        worker.submit(broken.clone());
        worker.submit(broken);
        worker.submit(demo::document());
        let done = wait(&worker);
        assert_eq!(done.generation, 3);
        assert!(!worker.busy(done.generation));
        let (model, scene) = done.result.unwrap();
        assert_eq!(model.bodies.len(), 1);
        assert!(!scene.triangles.is_empty());
    }

    #[test]
    fn failures_come_back_as_messages() {
        let mut worker = RebuildWorker::new(|| {});
        let mut doc = demo::document();
        // Suppressing a sketch its extrusion needs is a rebuild error, not a panic.
        doc.features[0].suppressed = true;
        worker.submit(doc);
        let err = wait(&worker).result.expect_err("rebuild should fail");
        assert!(!err.is_empty());
    }

    #[test]
    fn empty_part_rebuilds_to_empty_scene() {
        let mut worker = RebuildWorker::new(|| {});
        let doc = PartDocument::new("Empty", sm_document::LengthUnit::Mm);
        worker.submit(doc);
        let (model, scene) = wait(&worker).result.unwrap();
        assert!(model.bodies.is_empty());
        assert!(scene.triangles.is_empty() && scene.edges.is_empty());
    }
}
