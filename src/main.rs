// Animator App — main entry point.
//
// Phase A walking skeleton: one canvas, one layer, mouse stroke, PNG save.
// Built on eframe (winit + wgpu + egui) for fast iteration. GPU compute brush
// + multi-layer composite move in over later phases.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod input;
mod io;
mod timeline;
mod tools;
mod ui;
mod undo;

use app::AppState;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("animator_app=info,warn"),
    )
    .init();

    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: eframe::wgpu::PresentMode::Mailbox,
            ..Default::default()
        },
        viewport: egui::ViewportBuilder::default()
            .with_title("Animator")
            // Decorations OFF is required for true transparent windows on Win11
            // (DWM only gives an alpha surface to borderless windows). We draw
            // a custom title bar inside egui so the user keeps drag + window
            // controls.
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Animator",
        native_options,
        Box::new(|cc| Ok(Box::new(AppState::new(cc)))),
    )
}
