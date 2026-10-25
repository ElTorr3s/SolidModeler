//! SolidModeller desktop app.
//!
//! `solidmodeller [part.smp]` opens a part (or the demo part if none is
//! given; a path that doesn't exist yet starts the demo and saves there).

mod app;
mod browser;
mod camera;
mod demo;
mod icons;
mod mesh;
mod render;
mod theme;
mod viewcube;
mod worker;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (doc, path) = match args.as_slice() {
        [] => (demo::document(), None),
        [p] if p.starts_with('-') => {
            eprintln!("usage: solidmodeller [part.smp]");
            return ExitCode::FAILURE;
        }
        [p] => {
            let path = PathBuf::from(p);
            if path.exists() {
                match sm_format::open(&path) {
                    Ok(file) => (file.document, Some(path)),
                    Err(e) => {
                        eprintln!("error: {p}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                (demo::document(), Some(path))
            }
        }
        _ => {
            eprintln!("usage: solidmodeller [part.smp]");
            return ExitCode::FAILURE;
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SolidModeller")
            // Hyprland matches window rules on the Wayland app id.
            .with_app_id("solidmodeller")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let result = eframe::run_native(
        "SolidModeller",
        options,
        Box::new(move |cc| Ok(Box::new(app::SolidApp::new(cc, doc, path)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
