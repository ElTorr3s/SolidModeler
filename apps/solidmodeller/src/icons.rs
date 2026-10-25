//! Small vector icons for the model browser, painted rather than drawn from a
//! font so they look the same whatever fonts the system has.

use crate::theme::Palette;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Part,
    Folder,
    Plane,
    Axis,
    Point,
    Sketch,
    Extrude,
    Cut,
    EndOfPart,
}

const SIZE: f32 = 14.0;

/// Paints `icon` in the next 14×14 slot of `ui`. `dim` greys it out for
/// suppressed or rolled-back features.
pub fn icon(ui: &mut Ui, icon: Icon, palette: &Palette, dim: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::hover());
    if ui.is_rect_visible(rect) {
        let mut colors = Colors::new(palette);
        if dim {
            colors.dim(palette);
        }
        paint(ui.painter(), rect.shrink(1.0), icon, &colors);
    }
}

struct Colors {
    line: Color32,
    accent: Color32,
    fill: Color32,
    red: Color32,
}

impl Colors {
    fn new(p: &Palette) -> Self {
        Colors {
            line: p.foreground.mix(p.background, 0.25).egui(),
            accent: p.accent.egui(),
            fill: p.accent.mix(p.background, 0.55).egui(),
            red: p.error.egui(),
        }
    }

    fn dim(&mut self, p: &Palette) {
        let weak = p.foreground.mix(p.background, 0.6).egui();
        *self = Colors {
            line: weak,
            accent: weak,
            fill: p.background.mix(p.foreground, 0.15).egui(),
            red: weak,
        };
    }
}

fn paint(painter: &Painter, r: Rect, icon: Icon, c: &Colors) {
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let thin = Stroke::new(1.0, c.line);
    match icon {
        Icon::Part => {
            // An isometric block.
            let top = [at(0.5, 0.0), at(1.0, 0.25), at(0.5, 0.5), at(0.0, 0.25)];
            let left = [at(0.0, 0.25), at(0.5, 0.5), at(0.5, 1.0), at(0.0, 0.75)];
            let right = [at(0.5, 0.5), at(1.0, 0.25), at(1.0, 0.75), at(0.5, 1.0)];
            polygon(painter, &left, c.fill, thin);
            polygon(painter, &right, c.fill, thin);
            polygon(painter, &top, c.accent, thin);
        }
        Icon::Folder => {
            let tab = [at(0.0, 0.15), at(0.4, 0.15), at(0.5, 0.3), at(0.0, 0.3)];
            let body = [at(0.0, 0.3), at(1.0, 0.3), at(1.0, 0.9), at(0.0, 0.9)];
            polygon(painter, &tab, c.fill, thin);
            polygon(painter, &body, c.fill, thin);
        }
        Icon::Plane => {
            let pts = [at(0.25, 0.1), at(1.0, 0.1), at(0.75, 0.9), at(0.0, 0.9)];
            polygon(painter, &pts, c.fill, Stroke::new(1.0, c.accent));
        }
        Icon::Axis => {
            painter.line_segment([at(0.05, 0.95), at(0.95, 0.05)], Stroke::new(1.5, c.accent));
            painter.line_segment([at(0.95, 0.05), at(0.55, 0.1)], Stroke::new(1.5, c.accent));
            painter.line_segment([at(0.95, 0.05), at(0.9, 0.45)], Stroke::new(1.5, c.accent));
        }
        Icon::Point => {
            painter.circle(at(0.5, 0.5), r.width() * 0.22, c.accent, thin);
        }
        Icon::Sketch => {
            // A pencil over a sketch line.
            painter.line_segment([at(0.0, 0.95), at(1.0, 0.95)], thin);
            let body = [at(0.2, 0.85), at(0.15, 0.7), at(0.75, 0.1), at(0.9, 0.25)];
            polygon(painter, &body, c.accent, thin);
            painter.line_segment([at(0.2, 0.85), at(0.05, 0.9)], Stroke::new(1.5, c.line));
        }
        Icon::Extrude | Icon::Cut => {
            // A box with an arrow out of (extrude) or into (cut) its top.
            let base = [at(0.0, 0.55), at(0.6, 0.55), at(0.6, 1.0), at(0.0, 1.0)];
            polygon(painter, &base, c.fill, thin);
            let color = if icon == Icon::Cut { c.red } else { c.accent };
            let s = Stroke::new(1.5, color);
            let (from, to) = if icon == Icon::Cut {
                (at(0.82, 0.0), at(0.82, 0.75))
            } else {
                (at(0.82, 0.75), at(0.82, 0.0))
            };
            painter.line_segment([from, to], s);
            let dir = if icon == Icon::Cut { -1.0 } else { 1.0 };
            let head = r.height() * 0.22 * dir;
            painter.line_segment([to, to + vec2(-r.width() * 0.16, head)], s);
            painter.line_segment([to, to + vec2(r.width() * 0.16, head)], s);
        }
        Icon::EndOfPart => {
            painter.circle_filled(at(0.5, 0.5), r.width() * 0.5, c.red);
            let inner = Rect::from_center_size(at(0.5, 0.5), r.size() * 0.4);
            painter.rect_filled(inner, 0.0, Color32::WHITE);
        }
    }
}

fn polygon(painter: &Painter, pts: &[Pos2], fill: Color32, stroke: Stroke) {
    painter.add(Shape::convex_polygon(pts.to_vec(), fill, stroke));
}
