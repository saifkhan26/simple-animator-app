// Animator App — main entry point.
//
// Phase A walking skeleton: one canvas, one layer, mouse stroke, PNG save.
// Built on eframe (winit + wgpu + egui) for fast iteration. GPU compute brush
// + multi-layer composite move in over later phases.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod brush_library;
mod color;
mod doc;
mod input;
mod io;
mod krita_helper;
mod krita_link;
#[cfg(test)]
mod krita_stress;
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

    eframe::run_native(
        "Animator",
        native_options(),
        Box::new(|cc| Ok(Box::new(AppState::new(cc)))),
    )
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: eframe::wgpu::PresentMode::Mailbox,
            ..Default::default()
        },
        // eframe's defaults, spelled out: `ui::canvas_gpu` draws into egui's
        // render pass with a pipeline built for no MSAA and no depth buffer,
        // and dithers as egui does.
        multisampling: 0,
        depth_buffer: 0,
        dithering: ui::canvas_gpu::DITHERING,
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
        // Raw window messages: the Ctrl+C / X / V presses egui-winit swallows,
        // and the tablet driver's change notices.
        event_loop_builder: Some(Box::new(input::msg_hook::install)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    /// `ui::canvas_gpu` draws inside egui's render pass, so its pipeline has
    /// to match the pass eframe opens. A pipeline that doesn't is a wgpu
    /// validation error at the first draw — a silent abort in release —
    /// so changing any of these means changing `canvas_gpu` first.
    #[test]
    fn the_window_is_what_the_canvas_pipeline_is_built_for() {
        let o = super::native_options();
        assert_eq!(o.multisampling, 0, "canvas_gpu builds for one sample");
        assert_eq!(o.depth_buffer, 0, "canvas_gpu builds for no depth buffer");
        assert_eq!(o.stencil_buffer, 0, "canvas_gpu builds for no stencil");
        assert_eq!(o.dithering, crate::ui::canvas_gpu::DITHERING);
    }
}
