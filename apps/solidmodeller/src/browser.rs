//! The model browser: Inventor's feature tree panel.
//!
//! Like Inventor, a sketch consumed by a feature is shown nested under that
//! feature, and the End of Part marker sits between the active features and
//! the rolled-back ones.

use crate::icons::{icon, Icon};
use crate::theme::Palette;
use egui::{collapsing_header::CollapsingState, RichText, Ui};
use sm_document::{BodyOperation, FeatureId, FeatureKind, PartDocument};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserAction {
    Select(Option<FeatureId>),
    ToggleSuppress(FeatureId),
    /// Move the End of Part marker to just below this feature.
    EndOfPartBelow(FeatureId),
    EndOfPartToEnd,
}

/// A top-level browser row and the sketches nested under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub feature: FeatureId,
    pub children: Vec<FeatureId>,
    /// Below the End of Part marker.
    pub rolled_back: bool,
}

/// Arranges the history into browser rows.
pub fn tree(doc: &PartDocument) -> Vec<Node> {
    let eop = doc
        .end_of_part
        .and_then(|id| doc.features.iter().position(|f| f.id == id))
        .unwrap_or(doc.features.len());
    let mut nested = HashSet::new();
    let mut nodes = Vec::new();
    for (i, feature) in doc.features.iter().enumerate() {
        let children = match &feature.kind {
            FeatureKind::Extrude(e)
                if doc.feature(e.sketch).is_some() && nested.insert(e.sketch) =>
            {
                vec![e.sketch]
            }
            _ => Vec::new(),
        };
        nodes.push(Node {
            feature: feature.id,
            children,
            rolled_back: i >= eop,
        });
    }
    nodes.retain(|n| !nested.contains(&n.feature));
    nodes
}

const ORIGIN: [(Icon, &str); 7] = [
    (Icon::Plane, "YZ Plane"),
    (Icon::Plane, "XZ Plane"),
    (Icon::Plane, "XY Plane"),
    (Icon::Axis, "X Axis"),
    (Icon::Axis, "Y Axis"),
    (Icon::Axis, "Z Axis"),
    (Icon::Point, "Center Point"),
];

pub fn show(
    ui: &mut Ui,
    doc: &PartDocument,
    selected: Option<FeatureId>,
    palette: &Palette,
) -> Option<BrowserAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        icon(ui, Icon::Part, palette, false);
        ui.label(RichText::new(&doc.name).strong());
    });
    ui.indent("part", |ui| {
        CollapsingState::load_with_default_open(ui.ctx(), ui.scope_id().with("origin"), false)
            .show_header(ui, |ui| {
                icon(ui, Icon::Folder, palette, false);
                ui.label("Origin");
            })
            .body(|ui| {
                for (kind, name) in ORIGIN {
                    ui.horizontal(|ui| {
                        icon(ui, kind, palette, false);
                        ui.label(name);
                    });
                }
            });

        let nodes = tree(doc);
        let mut eop_drawn = false;
        for node in &nodes {
            if node.rolled_back && !eop_drawn {
                end_of_part(ui, palette, &mut action);
                eop_drawn = true;
            }
            let id = ui.scope_id().with(("feature", node.feature.0));
            if node.children.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(ui.spacing().indent);
                    row(
                        ui,
                        doc,
                        node.feature,
                        node.rolled_back,
                        selected,
                        palette,
                        &mut action,
                    );
                });
            } else {
                CollapsingState::load_with_default_open(ui.ctx(), id, false)
                    .show_header(ui, |ui| {
                        row(
                            ui,
                            doc,
                            node.feature,
                            node.rolled_back,
                            selected,
                            palette,
                            &mut action,
                        )
                    })
                    .body(|ui| {
                        for &child in &node.children {
                            row(
                                ui,
                                doc,
                                child,
                                node.rolled_back,
                                selected,
                                palette,
                                &mut action,
                            );
                        }
                    });
            }
        }
        if !eop_drawn {
            end_of_part(ui, palette, &mut action);
        }
    });
    action
}

fn end_of_part(ui: &mut Ui, palette: &Palette, action: &mut Option<BrowserAction>) {
    let response = ui
        .horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            icon(ui, Icon::EndOfPart, palette, false);
            ui.label(RichText::new("End of Part").color(palette.error.egui()))
        })
        .inner;
    response.context_menu(|ui| {
        if ui.button("Move EOP to End").clicked() {
            *action = Some(BrowserAction::EndOfPartToEnd);
            ui.close();
        }
    });
}

fn row(
    ui: &mut Ui,
    doc: &PartDocument,
    id: FeatureId,
    rolled_back: bool,
    selected: Option<FeatureId>,
    palette: &Palette,
    action: &mut Option<BrowserAction>,
) {
    let Some(feature) = doc.feature(id) else {
        return;
    };
    let kind = match &feature.kind {
        FeatureKind::Sketch(_) => Icon::Sketch,
        FeatureKind::Extrude(e) => match e.operation {
            BodyOperation::Cut => Icon::Cut,
            _ => Icon::Extrude,
        },
    };
    icon(ui, kind, palette, feature.suppressed || rolled_back);
    let mut text = RichText::new(&feature.name);
    if feature.suppressed {
        text = text.strikethrough().weak();
    } else if rolled_back {
        text = text.weak();
    }
    let response = ui.selectable_label(selected == Some(id), text);
    if response.clicked() {
        *action = Some(BrowserAction::Select(Some(id)));
    }
    response.context_menu(|ui| {
        let label = if feature.suppressed {
            "Unsuppress Features"
        } else {
            "Suppress Features"
        };
        if ui.button(label).clicked() {
            *action = Some(BrowserAction::ToggleSuppress(id));
            ui.close();
        }
        if ui.button("Move EOP below this").clicked() {
            *action = Some(BrowserAction::EndOfPartBelow(id));
            ui.close();
        }
    });
}

/// Applies a browser edit to the document. Returns true if geometry must be
/// rebuilt.
pub fn apply(doc: &mut PartDocument, action: BrowserAction) -> bool {
    match action {
        BrowserAction::Select(_) => false,
        BrowserAction::ToggleSuppress(id) => {
            match doc.features.iter_mut().find(|f| f.id == id) {
                Some(f) => f.suppressed = !f.suppressed,
                None => return false,
            }
            true
        }
        BrowserAction::EndOfPartBelow(id) => {
            // The marker is stored as the first feature below it. A feature
            // shown nested under a later one moves the marker below its parent.
            let parent = tree(doc)
                .into_iter()
                .find(|n| n.feature == id || n.children.contains(&id))
                .map_or(id, |n| n.feature);
            let Some(i) = doc.features.iter().position(|f| f.id == parent) else {
                return false;
            };
            doc.end_of_part = doc.features.get(i + 1).map(|f| f.id);
            true
        }
        BrowserAction::EndOfPartToEnd => {
            doc.end_of_part = None;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn sketches_nest_under_their_extrusion() {
        let doc = demo::document();
        let nodes = tree(&doc);
        assert_eq!(nodes.len(), 3);
        for node in &nodes {
            assert!(doc
                .feature(node.feature)
                .unwrap()
                .name
                .starts_with("Extrusion"));
            assert_eq!(node.children.len(), 1);
            assert!(!node.rolled_back);
        }
    }

    #[test]
    fn end_of_part_rolls_back_and_restores() {
        let mut doc = demo::document();
        let second_sketch = doc.features[2].id;
        assert!(apply(
            &mut doc,
            BrowserAction::EndOfPartBelow(second_sketch)
        ));
        // Sketch2 is nested under Extrusion2, so the marker goes below that.
        assert_eq!(doc.end_of_part, Some(doc.features[4].id));
        let rolled: Vec<_> = tree(&doc).iter().map(|n| n.rolled_back).collect();
        assert_eq!(rolled, [false, false, true]);
        assert_eq!(doc.active_features().count(), 4);

        apply(&mut doc, BrowserAction::EndOfPartToEnd);
        assert_eq!(doc.active_features().count(), 6);
    }

    #[test]
    fn suppress_toggles() {
        let mut doc = demo::document();
        let id = doc.features[5].id;
        apply(&mut doc, BrowserAction::ToggleSuppress(id));
        assert!(doc.feature(id).unwrap().suppressed);
        apply(&mut doc, BrowserAction::ToggleSuppress(id));
        assert!(!doc.feature(id).unwrap().suppressed);
    }
}
