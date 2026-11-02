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
    // An existing file is loaded on the app's worker thread, so a large
    // file shows the window (and a spinner) instead of a frozen start.
    let start = match args.as_slice() {
        [] => app::Start::Demo { save_to: None },
        [p] if p.starts_with('-') => {
            eprintln!("usage: solidmodeller [part.smp]");
            return ExitCode::FAILURE;
        }
        [p] => {
            let path = PathBuf::from(p);
            if path.exists() {
                app::Start::Open(path)
            } else {
                app::Start::Demo {
                    save_to: Some(path),
                }
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
        Box::new(move |cc| Ok(Box::new(app::SolidApp::new(cc, start)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
