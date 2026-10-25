//! Orthographic viewing camera with Inventor-style standard views.
//!
//! The model is Y-up like Inventor's default: Front looks at the XY plane
//! from +Z, Top looks down from +Y, Right looks from +X.

use glam::{Mat4, Quat, Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
    /// Inventor's Home view: isometric from front, top, right.
    Home,
}

impl StandardView {
    pub fn rotation(self) -> Quat {
        use std::f32::consts::FRAC_PI_2;
        match self {
            StandardView::Front => Quat::IDENTITY,
            StandardView::Back => Quat::from_rotation_y(2.0 * FRAC_PI_2),
            StandardView::Top => Quat::from_rotation_x(-FRAC_PI_2),
            StandardView::Bottom => Quat::from_rotation_x(FRAC_PI_2),
            StandardView::Right => Quat::from_rotation_y(FRAC_PI_2),
            StandardView::Left => Quat::from_rotation_y(-FRAC_PI_2),
            StandardView::Home => {
                // Elevation of a cube diagonal: atan(1 / sqrt(2)).
                let elevation = (1.0f32 / 2f32.sqrt()).atan();
                Quat::from_rotation_y(FRAC_PI_2 / 2.0) * Quat::from_rotation_x(-elevation)
            }
        }
    }
}

/// Seconds a view change takes to animate.
const VIEW_ANIMATION: f32 = 0.25;

#[derive(Debug, Clone)]
pub struct Camera {
    /// Camera-to-world rotation. The camera looks along its local -Z with +Y up.
    pub rotation: Quat,
    /// Point the camera orbits around and looks at.
    pub target: Vec3,
    /// Half the viewport height in model units (orthographic zoom).
    pub half_height: f32,
    /// Bounding sphere of the scene, for clip planes.
    pub scene_center: Vec3,
    pub scene_radius: f32,
    animation: Option<(Quat, Quat, f32)>,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            rotation: StandardView::Home.rotation(),
            target: Vec3::ZERO,
            half_height: 50.0,
            scene_center: Vec3::ZERO,
            scene_radius: 50.0,
            animation: None,
        }
    }
}

impl Camera {
    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    /// Distance from the target back to the eye: far enough that the whole
    /// scene is in front of the camera wherever the target has been panned to.
    fn eye_distance(&self) -> f32 {
        2.0 * (self.scene_radius + self.target.distance(self.scene_center)) + 1.0
    }

    pub fn eye(&self) -> Vec3 {
        self.target - self.forward() * self.eye_distance()
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.eye(), self.forward(), self.up())
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        let h = self.half_height;
        let w = h * aspect;
        Mat4::orthographic_rh(-w, w, -h, h, 0.0, 2.0 * self.eye_distance())
    }

    /// Model units per physical pixel for a viewport `height_px` tall.
    pub fn units_per_pixel(&self, height_px: f32) -> f32 {
        2.0 * self.half_height / height_px.max(1.0)
    }

    /// Point on the plane through the target, under viewport pixel `px`.
    /// `px` is relative to the viewport's top-left corner.
    pub fn point_at(&self, px: Vec2, size: Vec2) -> Vec3 {
        let k = self.units_per_pixel(size.y);
        let off = (px - size / 2.0) * k;
        self.target + self.right() * off.x - self.up() * off.y
    }

    /// Ray through pixel `px`: origin and (unit) direction.
    pub fn ray(&self, px: Vec2, size: Vec2) -> (Vec3, Vec3) {
        let p = self.point_at(px, size);
        (p - self.forward() * self.eye_distance(), self.forward())
    }

    /// Turntable orbit: horizontal drag spins about world Y, vertical drag
    /// tilts about the screen's horizontal axis.
    pub fn orbit(&mut self, delta_px: Vec2) {
        self.animation = None;
        let k = 0.008;
        let yaw = Quat::from_rotation_y(-delta_px.x * k);
        let pitch = Quat::from_rotation_x(-delta_px.y * k);
        self.rotation = (yaw * self.rotation * pitch).normalize();
    }

    pub fn pan(&mut self, delta_px: Vec2, height_px: f32) {
        let k = self.units_per_pixel(height_px);
        self.target -= self.right() * delta_px.x * k - self.up() * delta_px.y * k;
    }

    /// Zooms by `factor` (< 1 zooms in) keeping the point under `px` fixed.
    pub fn zoom_at(&mut self, factor: f32, px: Vec2, size: Vec2) {
        let factor = factor.clamp(0.2, 5.0);
        let new_half = (self.half_height * factor).clamp(1e-4, 1e7);
        let factor = new_half / self.half_height;
        let anchor = self.point_at(px, size);
        self.target = anchor + (self.target - anchor) * factor;
        self.half_height = new_half;
    }

    /// Zooms and pans so the scene fills the viewport, keeping the rotation.
    pub fn fit(&mut self, aspect: f32) {
        self.target = self.scene_center;
        let margin = 1.15;
        self.half_height = self.scene_radius * margin / aspect.clamp(1e-3, 1.0);
    }

    /// Starts animating to a standard view, then fits.
    pub fn go_to(&mut self, view: StandardView, now: f32) {
        self.go_to_rotation(view.rotation(), now);
    }

    pub fn go_to_rotation(&mut self, goal: Quat, now: f32) {
        self.animation = Some((self.rotation, goal, now));
    }

    /// Advances an animation. Returns true while one is running.
    pub fn animate(&mut self, now: f32) -> bool {
        let Some((from, to, start)) = self.animation else {
            return false;
        };
        let t = ((now - start) / VIEW_ANIMATION).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        self.rotation = from.slerp(to, eased).normalize();
        if t >= 1.0 {
            self.animation = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    #[test]
    fn standard_views_look_the_inventor_way() {
        let look = |v: StandardView| v.rotation() * Vec3::NEG_Z;
        let up = |v: StandardView| v.rotation() * Vec3::Y;
        assert!(close(look(StandardView::Front), Vec3::NEG_Z));
        assert!(close(look(StandardView::Top), Vec3::NEG_Y));
        // Top view keeps the front edge at the bottom of the screen.
        assert!(close(up(StandardView::Top), Vec3::NEG_Z));
        assert!(close(look(StandardView::Right), Vec3::NEG_X));
        assert!(close(look(StandardView::Left), Vec3::X));
        assert!(close(look(StandardView::Back), Vec3::Z));
        assert!(close(look(StandardView::Bottom), Vec3::Y));
        let home = look(StandardView::Home);
        assert!(close(home, Vec3::new(-1.0, -1.0, -1.0).normalize()));
    }

    #[test]
    fn zoom_keeps_cursor_point_fixed() {
        let mut cam = Camera::default();
        let size = Vec2::new(800.0, 600.0);
        let px = Vec2::new(100.0, 450.0);
        let before = cam.point_at(px, size);
        cam.zoom_at(0.5, px, size);
        assert!(close(before, cam.point_at(px, size)));
        assert!((cam.half_height - 25.0).abs() < 1e-4);
    }

    #[test]
    fn pan_follows_the_mouse() {
        let mut cam = Camera::default();
        let size = Vec2::new(800.0, 600.0);
        let grab = Vec2::new(300.0, 200.0);
        let point = cam.point_at(grab, size);
        let delta = Vec2::new(40.0, -25.0);
        cam.pan(delta, size.y);
        assert!(close(point, cam.point_at(grab + delta, size)));
    }

    #[test]
    fn scene_stays_inside_clip_volume() {
        let mut cam = Camera {
            scene_radius: 10.0,
            ..Camera::default()
        };
        cam.target = Vec3::new(30.0, 0.0, 0.0);
        let vp = cam.projection(1.0) * cam.view();
        for p in [Vec3::splat(10.0), Vec3::splat(-10.0)] {
            let clip = vp.project_point3(p);
            assert!((0.0..=1.0).contains(&clip.z), "{p} -> {clip}");
        }
    }

    #[test]
    fn animation_ends_on_goal() {
        let mut cam = Camera::default();
        cam.go_to(StandardView::Top, 0.0);
        assert!(cam.animate(0.1));
        cam.animate(1.0);
        assert!(!cam.animate(1.1));
        assert!(close(cam.forward(), Vec3::NEG_Y));
    }
}
