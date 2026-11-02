//! An Inventor-style ViewCube in the viewport corner, plus an axis triad.
//! Clicking a face turns the camera to that standard view.

use crate::camera::{Camera, StandardView};
use crate::theme::Palette;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use glam::Vec3;

const FACES: [(Vec3, StandardView, &str); 6] = [
    (Vec3::Z, StandardView::Front, "FRONT"),
    (Vec3::NEG_Z, StandardView::Back, "BACK"),
    (Vec3::Y, StandardView::Top, "TOP"),
    (Vec3::NEG_Y, StandardView::Bottom, "BOTTOM"),
    (Vec3::X, StandardView::Right, "RIGHT"),
    (Vec3::NEG_X, StandardView::Left, "LEFT"),
];

/// Half the cube's size on screen, in points.
const HALF: f32 = 32.0;

pub struct ViewCube {
    pub center: Pos2,
}

impl ViewCube {
    pub fn new(viewport: Rect) -> Self {
        ViewCube {
            center: viewport.right_top() + Vec2::new(-HALF * 2.2, HALF * 2.2),
        }
    }

    /// Area that captures the mouse, so clicks on the cube don't pick the model.
    pub fn rect(&self) -> Rect {
        Rect::from_center_size(self.center, Vec2::splat(HALF * 3.6))
    }

    /// Visible faces as screen polygons, back to front.
    fn faces(&self, camera: &Camera) -> Vec<([Pos2; 4], f32, StandardView, &'static str)> {
        let (right, up, toward) = (camera.right(), camera.up(), -camera.forward());
        let project = |p: Vec3| self.center + Vec2::new(p.dot(right), -p.dot(up)) * HALF;
        let mut faces: Vec<_> = FACES
            .iter()
            .filter_map(|&(n, view, label)| {
                let facing = n.dot(toward);
                if facing < 0.02 {
                    return None;
                }
                // Two axes spanning the face.
                let (a, b) = if n.x != 0.0 {
                    (Vec3::Y, Vec3::Z)
                } else if n.y != 0.0 {
                    (Vec3::X, Vec3::Z)
                } else {
                    (Vec3::X, Vec3::Y)
                };
                let corners = [n - a - b, n + a - b, n + a + b, n - a + b].map(project);
                Some((corners, facing, view, label))
            })
            .collect();
        faces.sort_by(|x, y| x.1.total_cmp(&y.1));
        faces
    }

    /// The face under `pos`, if any.
    pub fn hit(&self, camera: &Camera, pos: Pos2) -> Option<StandardView> {
        self.faces(camera)
            .into_iter()
            .rev()
            .find(|(poly, ..)| inside(poly, pos))
            .map(|(_, _, view, _)| view)
    }

    /// The Home button sitting above-left of the cube, like Inventor's.
    pub fn home_rect(&self) -> Rect {
        Rect::from_center_size(
            self.center + Vec2::new(-HALF * 1.5, -HALF * 1.5),
            Vec2::splat(18.0),
        )
    }

    pub fn paint(
        &self,
        painter: &Painter,
        camera: &Camera,
        palette: &Palette,
        hover: Option<Pos2>,
    ) {
        let fg = palette.foreground;
        let bg = palette.background;
        let hovered = hover.and_then(|p| self.hit(camera, p));
        let stroke = Stroke::new(1.0, fg.mix(bg, 0.45).egui());
        for (poly, facing, view, label) in self.faces(camera) {
            let fill = if Some(view) == hovered {
                palette.accent.mix(bg, 0.35)
            } else {
                // Faces turned towards the viewer read lighter, like a lit cube.
                bg.mix(fg, 0.10 + 0.12 * facing)
            };
            painter.add(Shape::convex_polygon(poly.to_vec(), fill.egui(), stroke));
            if facing > 0.35 {
                let mid = poly.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / 4.0;
                let alpha = ((facing - 0.35) / 0.4).clamp(0.0, 1.0);
                painter.text(
                    mid.to_pos2(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(9.0),
                    fg.egui().gamma_multiply(alpha),
                );
            }
        }

        let home = self.home_rect();
        let home_hovered = hover.is_some_and(|p| home.contains(p));
        let color = if home_hovered {
            palette.accent
        } else {
            fg.mix(bg, 0.3)
        }
        .egui();
        paint_house(painter, home, color);
    }

    /// X/Y/Z triad in the bottom-left corner.
    pub fn paint_triad(painter: &Painter, viewport: Rect, camera: &Camera, palette: &Palette) {
        let origin = viewport.left_bottom() + Vec2::new(40.0, -40.0);
        let (right, up) = (camera.right(), camera.up());
        let axes = [
            (Vec3::X, "X", palette.error),
            (Vec3::Y, "Y", crate::theme::Rgb(0x4c, 0xb0, 0x50)),
            (Vec3::Z, "Z", palette.accent),
        ];
        for (axis, label, color) in axes {
            let tip = origin + Vec2::new(axis.dot(right), -axis.dot(up)) * 26.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, color.egui()));
            painter.text(
                tip + (tip - origin).normalized() * 7.0,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(10.0),
                color.egui(),
            );
        }
    }
}

fn paint_house(painter: &Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let mid = rect.center().x;
    let eave = t + rect.height() * 0.45;
    painter.add(Shape::line(
        vec![Pos2::new(l, eave), Pos2::new(mid, t), Pos2::new(r, eave)],
        stroke,
    ));
    painter.add(Shape::line(
        vec![
            Pos2::new(l + 3.0, eave - 1.0),
            Pos2::new(l + 3.0, b),
            Pos2::new(r - 3.0, b),
            Pos2::new(r - 3.0, eave - 1.0),
        ],
        stroke,
    ));
}

/// Point-in-convex-polygon, either winding.
fn inside(poly: &[Pos2; 4], p: Pos2) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (poly[i], poly[(i + 1) % 4]);
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_view_shows_front_face_in_the_middle() {
        let mut camera = Camera::default();
        camera.go_to(StandardView::Front, 0.0);
        camera.animate(10.0);
        let cube = ViewCube::new(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)));
        assert_eq!(cube.hit(&camera, cube.center), Some(StandardView::Front));
    }

    #[test]
    fn home_view_shows_three_faces() {
        let camera = Camera::default();
        let cube = ViewCube::new(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)));
        let views: Vec<_> = cube.faces(&camera).iter().map(|f| f.2).collect();
        assert_eq!(views.len(), 3);
        for v in [StandardView::Front, StandardView::Top, StandardView::Right] {
            assert!(views.contains(&v));
        }
        // Slightly above centre is the top face.
        let above = cube.center + Vec2::new(0.0, -HALF * 0.6);
        assert_eq!(cube.hit(&camera, above), Some(StandardView::Top));
    }
}
