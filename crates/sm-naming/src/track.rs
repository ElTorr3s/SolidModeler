//! Follows a rebuild and carries face origins through every boolean.

use crate::naming::{name_body, BodyNaming};
use crate::tools::{extrude_tool, FaceOrigins};
use crate::Naming;
use sm_document::{FeatureId, FeatureKind, PartDocument};
use sm_kernel::{BodyChange, ExtrudeTool, RebuildObserver, LINEAR_TOLERANCE};
use std::sync::Arc;
use truck_modeling::Solid;

#[derive(Debug, Clone)]
pub(crate) struct BodyState {
    pub solid: Solid,
    pub origins: FaceOrigins,
}

impl BodyState {
    pub fn naming(&self) -> BodyNaming {
        name_body(&self.solid, &self.origins)
    }
}

/// One step of the rebuild, as the tracker logs it.
#[derive(Debug, Clone)]
pub(crate) enum Change {
    Created(usize, Arc<BodyState>),
    Modified(usize, Arc<BodyState>),
    Removed(usize),
}

impl Change {
    pub fn apply(&self, bodies: &mut Vec<Arc<BodyState>>) {
        match self {
            Change::Created(i, b) => bodies.insert((*i).min(bodies.len()), b.clone()),
            Change::Modified(i, b) => {
                if let Some(slot) = bodies.get_mut(*i) {
                    *slot = b.clone();
                }
            }
            Change::Removed(i) => {
                if *i < bodies.len() {
                    bodies.remove(*i);
                }
            }
        }
    }
}

/// A [`RebuildObserver`] that records where every face came from. It logs
/// each body change rather than copying all bodies per feature, so memory
/// grows with the number of booleans, not features times bodies; the state
/// at any point in the history is recovered by replaying the log.
pub(crate) struct Tracker<'d> {
    doc: &'d PartDocument,
    tool: FaceOrigins,
    pub bodies: Vec<Arc<BodyState>>,
    /// Every body change, in history order.
    pub changes: Vec<(FeatureId, Change)>,
}

impl<'d> Tracker<'d> {
    pub fn new(doc: &'d PartDocument) -> Self {
        Tracker {
            doc,
            tool: FaceOrigins::default(),
            bodies: Vec::new(),
            changes: Vec::new(),
        }
    }

    fn record(&mut self, feature: FeatureId, change: Change) {
        change.apply(&mut self.bodies);
        self.changes.push((feature, change));
    }
}

impl RebuildObserver for Tracker<'_> {
    fn tool_built(&mut self, feature: FeatureId, tool: &ExtrudeTool<'_>) {
        let profiles = match self.doc.feature(feature).map(|f| &f.kind) {
            Some(FeatureKind::Extrude(e)) => match self.doc.feature(e.sketch).map(|f| &f.kind) {
                Some(FeatureKind::Sketch(s)) => s.profiles.as_slice(),
                _ => &[],
            },
            _ => &[],
        };
        // The kernel always sweeps along +normal; a tool that ends at the
        // sketch plane came from a negative distance.
        let reversed = tool.start < 0.0 && (tool.start + tool.length).abs() <= LINEAR_TOLERANCE;
        self.tool = extrude_tool(
            feature,
            profiles,
            tool.region,
            tool.frame,
            reversed,
            tool.solid,
        );
    }

    fn body_created(&mut self, feature: FeatureId, body: usize, solid: &Solid) {
        let state = BodyState {
            solid: solid.clone(),
            origins: std::mem::take(&mut self.tool),
        };
        self.record(feature, Change::Created(body, Arc::new(state)));
    }

    fn body_modified(&mut self, feature: FeatureId, body: usize, change: &BodyChange<'_>) {
        let Some(state) = self.bodies.get(body) else {
            return;
        };
        let state = BodyState {
            origins: FaceOrigins::carried(change.after, &[&state.origins, &self.tool]),
            solid: change.after.clone(),
        };
        self.record(feature, Change::Modified(body, Arc::new(state)));
    }

    fn body_removed(&mut self, feature: FeatureId, body: usize) {
        self.record(feature, Change::Removed(body));
    }
}

pub(crate) fn naming(bodies: &[Arc<BodyState>]) -> Naming {
    Naming {
        bodies: bodies.iter().map(|b| b.naming()).collect(),
    }
}
