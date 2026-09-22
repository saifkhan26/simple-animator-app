//! Ctrl+C / Ctrl+X / Ctrl+V as ordinary key presses.
//!
//! egui-winit turns the clipboard chords into `Event::Copy` / `Event::Cut` /
//! `Event::Paste` and returns before it emits the key itself, so a binding
//! such as Ctrl+C never sees `key_pressed(C)`. Paste is worse: it is only sent
//! when the system clipboard holds *text*, so with an image or nothing on it
//! Ctrl+V leaves no trace in egui at all — and the selection clipboard lives
//! inside the app, where the system clipboard's contents don't matter.
//!
//! On Windows a winit message hook sees every key-down before egui-winit does
//! and records the keys it may swallow; `inject` puts back the presses that
//! never arrived. Elsewhere the Copy / Cut / Paste events are mapped back to
//! keys, which still misses a paste with no text on the clipboard.

/// Keys the message hook saw go down since the last frame. Only the handful
/// egui-winit can swallow, each recorded once, so this never grows.
#[cfg(target_os = "windows")]
static PRESSED: std::sync::Mutex<Vec<egui::Key>> = std::sync::Mutex::new(Vec::new());

/// Watch key-downs on the event loop. Passed as eframe's `event_loop_builder`.
#[cfg(target_os = "windows")]
pub fn install(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_C, VK_DELETE, VK_INSERT, VK_V, VK_X};
    use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN, WM_SYSKEYDOWN};
    use winit::platform::windows::EventLoopBuilderExtWindows;

    builder.with_msg_hook(|msg| {
        // SAFETY: winit hands the hook a pointer to the `MSG` it just read.
        let msg = unsafe { &*(msg as *const MSG) };
        // SYSKEYDOWN too: holding Alt turns Ctrl+Alt+V into one.
        if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
            return false;
        }
        // Every key egui-winit can turn into a clipboard event: Ctrl+X / C / V
        // and the Windows-only Shift+Delete, Ctrl+Insert, Shift+Insert.
        let vk = msg.wParam.0 as u16;
        let key = match vk {
            _ if vk == VK_C.0 => egui::Key::C,
            _ if vk == VK_X.0 => egui::Key::X,
            _ if vk == VK_V.0 => egui::Key::V,
            _ if vk == VK_INSERT.0 => egui::Key::Insert,
            _ if vk == VK_DELETE.0 => egui::Key::Delete,
            _ => return false,
        };
        if let Ok(mut pressed) = PRESSED.lock() {
            if !pressed.contains(&key) {
                pressed.push(key);
            }
        }
        // Never eat the message: winit and egui-winit still see it as usual.
        false
    });
}

#[cfg(not(target_os = "windows"))]
pub fn install(_builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {}

#[cfg(target_os = "windows")]
fn take_pressed(_raw: &egui::RawInput) -> Vec<egui::Key> {
    PRESSED
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default()
}

#[cfg(not(target_os = "windows"))]
fn take_pressed(raw: &egui::RawInput) -> Vec<egui::Key> {
    raw.events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Cut => Some(egui::Key::X),
            egui::Event::Copy => Some(egui::Key::C),
            egui::Event::Paste(_) => Some(egui::Key::V),
            _ => None,
        })
        .collect()
}

/// Add a key-down for every clipboard key egui-winit swallowed this frame.
/// Called from `raw_input_hook`, before egui reads the input.
///
/// The recorded keys include ones egui-winit passed through untouched (a
/// plain C, say), so a key already pressed this frame is left alone rather
/// than doubled. The clipboard events themselves stay, so text fields still
/// copy and paste as before.
pub fn inject(raw: &mut egui::RawInput) {
    for key in take_pressed(raw) {
        let arrived = raw.events.iter().any(|e| {
            matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)
        });
        if !arrived {
            raw.events.push(egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: raw.modifiers,
            });
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::input::shortcuts::{Action, ShortcutMap};

    fn actions_for(raw: egui::RawInput) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        let _ = ctx.run(raw, |ctx| out = ShortcutMap::default().poll_actions(ctx));
        out
    }

    // One test, not several: `PRESSED` is a process-wide static and the test
    // harness runs tests in parallel.
    #[test]
    fn swallowed_clipboard_keys_reach_the_shortcuts() {
        // Ctrl+V with no text on the clipboard: egui-winit sends nothing.
        PRESSED.lock().unwrap().push(egui::Key::V);
        let mut raw = egui::RawInput {
            modifiers: egui::Modifiers::CTRL,
            ..Default::default()
        };
        inject(&mut raw);
        assert!(PRESSED.lock().unwrap().is_empty(), "drained each frame");
        assert!(actions_for(raw).contains(&Action::SelectionPaste));

        // Ctrl+C: egui-winit sends only the clipboard event, which stays.
        PRESSED.lock().unwrap().push(egui::Key::C);
        let mut raw = egui::RawInput {
            modifiers: egui::Modifiers::CTRL,
            events: vec![egui::Event::Copy],
            ..Default::default()
        };
        inject(&mut raw);
        assert!(raw.events.contains(&egui::Event::Copy));
        assert!(actions_for(raw).contains(&Action::SelectionCopy));

        // A plain C already arrived as a key: not pressed a second time.
        PRESSED.lock().unwrap().push(egui::Key::C);
        let plain_c = egui::Event::Key {
            key: egui::Key::C,
            physical_key: Some(egui::Key::C),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut raw = egui::RawInput {
            events: vec![plain_c.clone()],
            ..Default::default()
        };
        inject(&mut raw);
        assert_eq!(raw.events, vec![plain_c]);
    }
}
