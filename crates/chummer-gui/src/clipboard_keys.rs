//! Copy, cut and paste with the classic keys: Ctrl+Insert, Shift+Delete
//! and Shift+Insert, next to Ctrl/Cmd+C, X and V.
//!
//! egui-winit understands those keys only on Windows. Desktops that map
//! their own shortcuts onto them need them everywhere: Omarchy's
//! Super+C/Super+V send Ctrl+Insert/Shift+Insert, and many Linux and
//! Windows users type them directly. On macOS Cmd+C/V already work.

use eframe::egui;

/// Turns the classic keys into egui's copy, cut and paste events. Paste
/// needs the clipboard text, which only the window backend can read: it is
/// asked for with `ViewportCommand::RequestPaste` and arrives as a paste
/// event on the next frame.
#[derive(Default)]
pub struct ClipboardKeys {
    paste: bool,
}

/// What a key press means, if it is one of the classic keys.
pub fn classic(key: egui::Key, m: egui::Modifiers) -> Option<egui::Event> {
    // Windows: egui-winit already turns them into copy, cut and paste.
    if cfg!(target_os = "windows") || m.alt {
        return None;
    }
    match key {
        egui::Key::Insert if m.ctrl && !m.shift => Some(egui::Event::Copy),
        egui::Key::Delete if m.shift && !m.ctrl => Some(egui::Event::Cut),
        egui::Key::Insert if m.shift && !m.ctrl => Some(egui::Event::Paste(String::new())),
        _ => None,
    }
}

impl egui::Plugin for ClipboardKeys {
    fn debug_name(&self) -> &'static str {
        "chummer-rs clipboard keys"
    }

    fn input_hook(&mut self, input: &mut egui::RawInput) {
        for e in &mut input.events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { continue };
            match classic(*key, *modifiers) {
                Some(egui::Event::Paste(_)) => self.paste = true,
                Some(ev) => *e = ev,
                None => {}
            }
        }
    }

    fn on_begin_pass(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.paste) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_keys_copy_cut_and_paste() {
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        let shift = egui::Modifiers::SHIFT;
        if cfg!(target_os = "windows") {
            assert_eq!(classic(egui::Key::Insert, ctrl), None, "egui-winit does it");
            return;
        }
        assert_eq!(classic(egui::Key::Insert, ctrl), Some(egui::Event::Copy));
        assert_eq!(classic(egui::Key::Delete, shift), Some(egui::Event::Cut));
        assert!(matches!(classic(egui::Key::Insert, shift), Some(egui::Event::Paste(_))));
        // Plain Insert and Delete stay keys, and so does Ctrl+Shift+Insert.
        assert_eq!(classic(egui::Key::Insert, egui::Modifiers::NONE), None);
        assert_eq!(classic(egui::Key::Delete, egui::Modifiers::NONE), None);
        assert_eq!(classic(egui::Key::Insert, ctrl | shift), None);
    }

    /// Ctrl+Insert copies a text field's selection, in a real egui pass;
    /// without the plugin it does not (egui-winit leaves it a plain key off
    /// Windows).
    #[test]
    fn ctrl_insert_copies_the_selection() {
        if cfg!(target_os = "windows") {
            return;
        }
        assert!(copies(true));
        assert!(!copies(false));
    }

    fn copies(plugin: bool) -> bool {
        let ctx = egui::Context::default();
        if plugin {
            ctx.add_plugin(ClipboardKeys::default());
        }
        let mut text = String::from("Argentum");
        let id = egui::Id::new("field");
        let frame = |ctx: &egui::Context, events: Vec<egui::Event>, text: &mut String| {
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let out = egui::TextEdit::singleline(text).id(id).show(ui);
                    if !out.response.has_focus() {
                        out.response.request_focus();
                    }
                    let mut state = out.state;
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(4))));
                    state.store(ui.ctx(), id);
                });
            })
        };
        frame(&ctx, vec![], &mut text);
        frame(&ctx, vec![], &mut text);
        let ctrl = egui::Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), ..Default::default() };
        let key = egui::Event::Key { key: egui::Key::Insert, physical_key: None, pressed: true, repeat: false, modifiers: ctrl };
        let out = frame(&ctx, vec![key], &mut text);
        out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "Arge"))
    }
}
