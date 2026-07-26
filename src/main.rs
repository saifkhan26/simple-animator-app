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
#[cfg(target_os = "windows")]
mod platform;
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
        // Always open maximized, so don't let eframe restore the last native
        // window rect — it defaults to true and would fight `with_maximized`.
        // Panel layout is separate (egui memory) and still persists.
        persist_window: false,
        // Keep UI state beside shortcuts.toml rather than in eframe's own data
        // dir: one folder to look in, one file to delete to reset the layout.
        persistence_path: input::shortcuts::config_dir().map(|d| d.join("ui.ron")),
        viewport: egui::ViewportBuilder::default()
            .with_title("Animator")
            .with_app_id("animator-app")
            // Frameless (no caption buttons / title). On Windows we re-apply the
            // default rounded corners + border via DWM in `platform`. Transparent
            // so the canvas backdrop alpha shows through.
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            // Not sufficient on its own for a frameless window — winit creates
            // it at `inner_size` and the creation-time flag is lost. The maximize
            // that actually lands is the first-frame `ViewportCommand::Maximized`
            // in `AppState::update`; keep both so platforms where this does work
            // skip the startup resize flash.
            .with_maximized(true)
            // Size used once un-maximized.
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
