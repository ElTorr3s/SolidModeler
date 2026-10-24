//! Follows a rebuild and carries face origins through every boolean.

use crate::naming::{name_body, BodyNaming};
use crate::tools::{extrude_tool, FaceOrigins};
use crate::Naming;
use sm_document::{FeatureId, FeatureKind, PartDocument};
use sm_kernel::{BodyChange, ExtrudeTool, RebuildObserver, LINEAR_TOLERANCE};
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

/// A [`RebuildObserver`] that records where every face came from, and the
/// bodies as they stood after each feature.
pub(crate) struct Tracker<'d> {
    doc: &'d PartDocument,
    tool: FaceOrigins,
    bodies: Vec<BodyState>,
    /// Bodies after each feature that changed them, in history order.
    pub states: Vec<(FeatureId, Vec<BodyState>)>,
}

impl<'d> Tracker<'d> {
    pub fn new(doc: &'d PartDocument) -> Self {
        Tracker {
            doc,
            tool: FaceOrigins::default(),
            bodies: Vec::new(),
            states: Vec::new(),
        }
    }

    fn record(&mut self, feature: FeatureId) {
        match self.states.last_mut() {
            Some((f, bodies)) if *f == feature => bodies.clone_from(&self.bodies),
            _ => self.states.push((feature, self.bodies.clone())),
        }
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
        self.bodies.insert(body.min(self.bodies.len()), state);
        self.record(feature);
    }

    fn body_modified(&mut self, feature: FeatureId, body: usize, change: &BodyChange<'_>) {
        let Some(state) = self.bodies.get_mut(body) else {
            return;
        };
        state.origins = FaceOrigins::carried(change.after, &[&state.origins, &self.tool]);
        state.solid = change.after.clone();
        self.record(feature);
    }

    fn body_removed(&mut self, feature: FeatureId, body: usize) {
        if body < self.bodies.len() {
            self.bodies.remove(body);
        }
        self.record(feature);
    }
}

pub(crate) fn naming(bodies: &[BodyState]) -> Naming {
    Naming {
        bodies: bodies.iter().map(BodyState::naming).collect(),
    }
}
