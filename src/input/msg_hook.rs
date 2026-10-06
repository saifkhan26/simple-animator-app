//! The event loop's message hook. winit takes a single `with_msg_hook`, so
//! everything that needs to see raw window messages before egui-winit does
//! is called from this one.

/// Passed as eframe's `event_loop_builder`.
#[cfg(target_os = "windows")]
pub fn install(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
    use windows::Win32::UI::WindowsAndMessaging::MSG;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    builder.with_msg_hook(|msg| {
        // SAFETY: winit hands the hook a pointer to the `MSG` it just read.
        let msg = unsafe { &*(msg as *const MSG) };
        super::clipboard_keys::on_msg(msg);
        super::tablet::on_msg(msg);
        // Never eat the message: winit and egui-winit still see it as usual.
        false
    });
}

#[cfg(not(target_os = "windows"))]
pub fn install(_builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {}
