//! Follows a rebuild and carries face origins through every feature.

use crate::naming::name_model;
use crate::tools::{extrude_tool, FaceOrigins};
use crate::Naming;
use sm_document::{FeatureId, FeatureKind, PartDocument};
use sm_kernel::{Body, ExtrudeTool, RebuildObserver};
use std::sync::Arc;
use truck_modeling::Solid;

#[derive(Debug, Clone)]
pub(crate) struct BodyState {
    pub solid: Solid,
    pub origins: FaceOrigins,
}

/// True if two solids are the same truck object: same faces, same ids.
/// Bodies a feature doesn't touch are kept as they were.
fn same_solid(a: &Solid, b: &Solid) -> bool {
    let ids = |s: &Solid| -> Vec<_> {
        s.boundaries()
            .iter()
            .flat_map(|shell| shell.face_iter().map(|f| f.id()))
            .collect()
    };
    ids(a) == ids(b)
}

/// A [`RebuildObserver`] that records where every face came from, and the
/// bodies after each feature. Bodies a feature leaves alone are shared
/// between states, so each state costs one pointer per body plus the bodies
/// that feature actually changed.
pub(crate) struct Tracker<'d> {
    doc: &'d PartDocument,
    /// Origins of every tool the current feature has built so far.
    tools: FaceOrigins,
    pub bodies: Vec<Arc<BodyState>>,
    /// Bodies after each feature that built geometry, in history order.
    pub states: Vec<(FeatureId, Vec<Arc<BodyState>>)>,
}

impl<'d> Tracker<'d> {
    pub fn new(doc: &'d PartDocument) -> Self {
        Tracker {
            doc,
            tools: FaceOrigins::default(),
            bodies: Vec::new(),
            states: Vec::new(),
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
        let reversed = tool.start < 0.0 && (tool.start + tool.length).abs() <= tool.tol.linear;
        let origins = extrude_tool(
            feature,
            profiles,
            tool.region,
            tool.frame,
            reversed,
            tool.solid,
            tool.tol.linear,
        );
        self.tools.extend(&origins);
    }

    fn bodies_changed(&mut self, feature: FeatureId, bodies: &[Body]) {
        // Every face of the new bodies comes from a face of an old body or
        // of one of this feature's tools.
        let mut sources = std::mem::take(&mut self.tools);
        for old in &self.bodies {
            sources.extend(&old.origins);
        }
        let next = bodies
            .iter()
            .map(|body| {
                match self
                    .bodies
                    .iter()
                    .find(|old| same_solid(&old.solid, &body.solid))
                {
                    Some(old) => old.clone(),
                    None => Arc::new(BodyState {
                        origins: FaceOrigins::carried(&body.solid, &sources),
                        solid: body.solid.clone(),
                    }),
                }
            })
            .collect();
        self.bodies = next;
        self.states.push((feature, self.bodies.clone()));
    }
}

pub(crate) fn naming(bodies: &[Arc<BodyState>]) -> Naming {
    let bodies: Vec<_> = bodies.iter().map(|b| (&b.solid, &b.origins)).collect();
    Naming {
        bodies: name_model(&bodies),
    }
}
