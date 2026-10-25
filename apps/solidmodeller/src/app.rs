use crate::browser::{self, BrowserAction};
use crate::camera::{Camera, StandardView};
use crate::mesh::SceneMesh;
use crate::render::{Uniforms, ViewportRenderer};
use crate::theme::{self, ColorScheme, Palette, ThemeEvent};
use crate::viewcube::ViewCube;
use crate::worker::RebuildWorker;
use egui::{Key, Modifiers, PointerButton, Sense};
use glam::Vec2;
use sm_document::{FeatureId, PartDocument};
use sm_kernel::Model;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// Something waiting on the user's answer to "save changes?".
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Close,
    Open(PathBuf),
}

pub struct SolidApp {
    doc: PartDocument,
    /// Where Ctrl+S saves. `None` for the built-in demo part.
    path: Option<PathBuf>,
    modified: bool,

    model: Option<Model>,
    scene: SceneMesh,
    rebuild_error: Option<String>,
    worker: RebuildWorker,
    /// Generation of the last rebuild result received.
    received: u64,
    status: Option<String>,
    /// An action held back until unsaved changes are saved or discarded.
    pending: Option<Pending>,
    /// Set once the user chose to close, so the close isn't intercepted again.
    closing: bool,
    title: String,

    camera: Camera,
    fit_on_next_frame: bool,
    renderer: ViewportRenderer,
    /// Inputs of the last rendered frame; an unchanged frame isn't redrawn.
    last_frame: Option<(Vec<u8>, [u32; 2], u64)>,
    texture: Option<egui::TextureId>,
    mesh_version: u64,

    selected_feature: Option<FeatureId>,

    theme_rx: Receiver<ThemeEvent>,
    omarchy: Option<Palette>,
    scheme: ColorScheme,
    palette: Palette,
}

impl SolidApp {
    pub fn new(cc: &eframe::CreationContext<'_>, doc: PartDocument, path: Option<PathBuf>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("eframe is built with the wgpu renderer");
        let (theme_tx, theme_rx) = channel();
        let ctx = cc.egui_ctx.clone();
        theme::watch(theme_tx, move || ctx.request_repaint());

        let system_dark = cc.egui_ctx.system_theme() != Some(egui::Theme::Light);
        let palette = Palette::fallback(system_dark);
        cc.egui_ctx.set_visuals(palette.visuals());

        let ctx = cc.egui_ctx.clone();
        let worker = RebuildWorker::new(move || ctx.request_repaint());
        let mut app = SolidApp {
            doc,
            path,
            modified: false,
            model: None,
            scene: SceneMesh::default(),
            rebuild_error: None,
            worker,
            received: 0,
            status: None,
            pending: None,
            closing: false,
            title: String::new(),
            camera: Camera::default(),
            fit_on_next_frame: true,
            renderer: ViewportRenderer::new(&render_state.device),
            last_frame: None,
            texture: None,
            mesh_version: 0,
            selected_feature: None,
            theme_rx,
            omarchy: None,
            scheme: ColorScheme::NoPreference,
            palette,
        };
        app.rebuild();
        app
    }

    fn rebuild(&mut self) {
        self.worker.submit(self.doc.clone());
    }

    fn rebuilding(&self) -> bool {
        self.worker.busy(self.received)
    }

    fn poll_background(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let mut theme_changed = false;
        while let Ok(event) = self.theme_rx.try_recv() {
            match event {
                ThemeEvent::Omarchy(p) => self.omarchy = p,
                ThemeEvent::Scheme(s) => self.scheme = s,
            }
            theme_changed = true;
        }
        if theme_changed {
            let system_dark = ctx.system_theme() != Some(egui::Theme::Light);
            let palette = theme::resolve(self.omarchy.as_ref(), self.scheme, system_dark);
            if palette != self.palette {
                ctx.set_visuals(palette.visuals());
                self.palette = palette;
            }
        }

        if let Some(done) = self.worker.poll() {
            self.received = done.generation;
            match done.result {
                Ok((model, scene)) => {
                    let first = self.model.is_none();
                    if let Some(rs) = frame.wgpu_render_state() {
                        self.renderer.set_mesh(&rs.device, &scene);
                    }
                    self.camera.scene_center = scene.center;
                    self.camera.scene_radius = scene.radius;
                    self.scene = scene;
                    self.model = Some(model);
                    self.mesh_version += 1;
                    self.rebuild_error = None;
                    self.fit_on_next_frame |= first;
                }
                // Keep showing the last good geometry, like Inventor does
                // while a feature is sick.
                Err(e) => self.rebuild_error = Some(e),
            }
        }
    }

    fn selected_body(&self) -> Option<u32> {
        let id = self.selected_feature?;
        self.scene
            .bodies
            .iter()
            .position(|&f| f == id)
            .map(|i| i as u32)
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.status = Some("Start with a file path to save (solidmodeller part.smp)".into());
            return;
        };
        // Only cache geometry that matches the document being saved.
        let step = match (
            &self.model,
            self.rebuilding() || self.rebuild_error.is_some(),
        ) {
            (Some(model), false) => Some(sm_step::export(model, &file_label(&path))),
            _ => None,
        };
        self.status = Some(match sm_format::save(&path, &self.doc, step.as_deref()) {
            Ok(()) => {
                self.modified = false;
                format!("Saved {}", path.display())
            }
            Err(e) => format!("Save failed: {e}"),
        });
    }

    /// Opens `path`, asking first if there are unsaved changes.
    fn request_open(&mut self, path: PathBuf) {
        if self.modified {
            self.pending = Some(Pending::Open(path));
        } else {
            self.open(path);
        }
    }

    fn open(&mut self, path: PathBuf) {
        match sm_format::open(&path) {
            Ok(file) => {
                self.doc = file.document;
                self.path = Some(path);
                self.modified = false;
                self.model = None;
                self.rebuild_error = None;
                self.selected_feature = None;
                self.status = None;
                self.rebuild();
            }
            Err(e) => self.status = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    self.save();
                    ui.close();
                }
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                let now = ui.input(|i| i.time) as f32;
                if ui
                    .add(egui::Button::new("Home View").shortcut_text("F6"))
                    .clicked()
                {
                    self.camera.go_to(StandardView::Home, now);
                    self.fit_on_next_frame = true;
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Zoom All").shortcut_text("Home"))
                    .clicked()
                {
                    self.fit_on_next_frame = true;
                    ui.close();
                }
                ui.separator();
                for (name, view) in [
                    ("Front", StandardView::Front),
                    ("Top", StandardView::Top),
                    ("Right", StandardView::Right),
                    ("Back", StandardView::Back),
                    ("Bottom", StandardView::Bottom),
                    ("Left", StandardView::Left),
                ] {
                    if ui.button(name).clicked() {
                        self.camera.go_to(view, now);
                        ui.close();
                    }
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let file = self
                .path
                .as_ref()
                .map_or("Demo part".into(), |p| p.display().to_string());
            ui.label(format!("{file}{}", if self.modified { " •" } else { "" }));
            ui.separator();
            if self.rebuilding() {
                ui.spinner();
                ui.label("Rebuilding");
            } else if let Some(e) = &self.rebuild_error {
                ui.colored_label(self.palette.error.egui(), format!("Rebuild failed: {e}"));
            } else {
                let n = self.scene.bodies.len();
                ui.label(format!("{n} {}", if n == 1 { "body" } else { "bodies" }));
            }
            if let Some(s) = &self.status {
                ui.separator();
                ui.label(s);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("Theme: {}", self.palette.name));
                ui.separator();
                ui.weak("MMB pan · Shift+MMB orbit · wheel zoom · F6 home");
            });
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let size = Vec2::new(rect.width(), rect.height());
        let now = ui.input(|i| i.time) as f32;
        let cube = ViewCube::new(rect);
        let hover = response.hover_pos();
        let local = |p: egui::Pos2| Vec2::new(p.x - rect.left(), p.y - rect.top());

        // Wait for a real size: a fit against a 0-pixel viewport is useless.
        if self.fit_on_next_frame && self.model.is_some() && size.x >= 1.0 && size.y >= 1.0 {
            self.camera.fit(size.x / size.y.max(1.0));
            self.fit_on_next_frame = false;
        }

        // Navigation, Inventor style.
        let (shift, f4) = ui.input(|i| (i.modifiers.shift, i.key_down(Key::F4)));
        let delta = response.drag_delta();
        let delta = Vec2::new(delta.x, delta.y);
        if response.dragged_by(PointerButton::Middle) {
            if shift {
                self.camera.orbit(delta);
            } else {
                self.camera.pan(delta, size.y);
            }
        } else if response.dragged_by(PointerButton::Primary) && f4 {
            self.camera.orbit(delta);
        }
        if let Some(pos) = hover {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (-scroll * 0.002).exp() / pinch;
            if (factor - 1.0).abs() > 1e-4 {
                self.camera.zoom_at(factor, local(pos), size);
            }
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if cube.home_rect().contains(pos) {
                    self.camera.go_to(StandardView::Home, now);
                    self.fit_on_next_frame = true;
                } else if let Some(view) = cube.hit(&self.camera, pos) {
                    self.camera.go_to(view, now);
                } else if !cube.rect().contains(pos) {
                    let (origin, dir) = self.camera.ray(local(pos), size);
                    self.selected_feature = self
                        .scene
                        .pick(origin, dir)
                        .and_then(|b| self.scene.bodies.get(b as usize).copied());
                }
            }
        }
        if self.camera.animate(now) {
            ui.ctx().request_repaint();
        }

        // Render only when an input of the frame changed.
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            (rect.width() * ppp).round() as u32,
            (rect.height() * ppp).round() as u32,
        ];
        let uniforms = self.uniforms(size);
        let key = (
            bytemuck::bytes_of(&uniforms).to_vec(),
            px,
            self.mesh_version,
        );
        if self.last_frame.as_ref() != Some(&key) {
            if let Some(rs) = frame.wgpu_render_state() {
                let mut egui_renderer = rs.renderer.write();
                self.texture = Some(self.renderer.render(
                    &rs.device,
                    &rs.queue,
                    &mut egui_renderer,
                    px,
                    &uniforms,
                ));
            }
            self.last_frame = Some(key);
        }

        let painter = ui.painter_at(rect);
        if let Some(texture) = self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture, rect, uv, egui::Color32::WHITE);
        }
        cube.paint(&painter, &self.camera, &self.palette, hover);
        ViewCube::paint_triad(&painter, rect, &self.camera, &self.palette);
    }

    fn uniforms(&self, size: Vec2) -> Uniforms {
        let p = &self.palette;
        let cam = &self.camera;
        let vp = cam.projection(size.x / size.y.max(1.0)) * cam.view();
        let mut u = Uniforms::new(vp);
        let rgba = |c: theme::Rgb| {
            let [r, g, b] = c.linear();
            [r, g, b, 1.0]
        };
        // Key light over the viewer's upper-left shoulder, moving with the view.
        let light = (cam.up() * 0.6 - cam.right() * 0.3 - cam.forward()).normalize();
        u.light = [light.x, light.y, light.z, 0.0];
        let eye = -cam.forward();
        u.eye = [eye.x, eye.y, eye.z, 0.0];
        // Inventor-like neutral body, tinted a touch towards the theme.
        let body = if p.dark {
            theme::Rgb(0xb4, 0xb8, 0xbe).mix(p.foreground, 0.15)
        } else {
            theme::Rgb(0xa8, 0xac, 0xb2).mix(p.foreground, 0.08)
        };
        u.body = rgba(body);
        u.edge = rgba(if p.dark {
            p.background.mix(theme::Rgb(0, 0, 0), 0.4)
        } else {
            p.foreground.mix(p.background, 0.15)
        });
        u.highlight = rgba(p.accent);
        // Gentle vertical gradient from the theme background.
        u.bg_top = rgba(p.background.mix(p.foreground, 0.02));
        u.bg_bottom = rgba(
            p.background
                .mix(p.foreground, if p.dark { 0.14 } else { 0.10 }),
        );
        u.selected[0] = self.selected_body().unwrap_or(u32::MAX);
        u
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time) as f32;
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F6)) {
            self.camera.go_to(StandardView::Home, now);
            self.fit_on_next_frame = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            self.fit_on_next_frame = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.selected_feature = None;
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .find_map(|f| Some(f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()))
        });
        if let Some(path) = dropped {
            self.request_open(path);
        }
        // Like Inventor, closing with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && self.modified && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }
    }
}

impl eframe::App for SolidApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_background(ctx, frame);
        self.shortcuts(ctx);
        let title = format!(
            "{}{} — SolidModeller",
            self.doc.name,
            if self.modified { " *" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        let mut action = None;
        egui::Panel::left("browser")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.heading("Model");
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    action = browser::show(ui, &self.doc, self.selected_feature, &self.palette);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport(ui, frame));

        if let Some(action) = action {
            if let BrowserAction::Select(id) = action {
                self.selected_feature = id;
            }
            if browser::apply(&mut self.doc, action) {
                self.modified = true;
                self.rebuild();
            }
        }
        self.unsaved_changes_dialog(ui);
    }
}

impl SolidApp {
    fn unsaved_changes_dialog(&mut self, ui: &mut egui::Ui) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("unsaved-changes")).show(ui.ctx(), |ui| {
            ui.heading("Save changes?");
            ui.label(format!("{} has unsaved changes.", self.doc.name));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if self.path.is_some() && ui.button("Save").clicked() {
                    choice = Some(true);
                }
                if ui.button("Don't Save").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.pending = None;
                }
            });
        });
        if modal.should_close() {
            self.pending = None;
        }
        let Some(save) = choice else {
            return;
        };
        if save {
            self.save();
            if self.modified {
                // Save failed; the status bar says why. Stay put.
                self.pending = None;
                return;
            }
        }
        self.pending = None;
        self.modified = false;
        match pending {
            Pending::Close => {
                self.closing = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::Open(path) => self.open(path),
        }
    }
}

fn file_label(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(|| "part".into(), |n| n.to_string_lossy().into_owned())
}
