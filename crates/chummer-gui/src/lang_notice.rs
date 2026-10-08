//! The incomplete-translation notice: switching to a language other than
//! English says that chummer-rs's own texts are only partly translated.
//! It does not block (a small window in a corner) and, once dismissed,
//! is not shown again for that language (`translation_notice_seen=` in
//! gui.ini, see [`crate::prefs`]).

use eframe::egui;

use crate::App;

/// Whether switching to `code` shows the notice.
pub fn needs_notice(code: &str, seen: &[String]) -> bool {
    !code.to_ascii_lowercase().starts_with("en") && !seen.iter().any(|s| s.eq_ignore_ascii_case(code))
}

/// The notice's text (also shown on the setup's language step).
pub fn notice_text(lang: &chummer_core::lang::Language) -> String {
    lang.tr("Chummer's game data, names and character sheets are translated. chummer-rs's own screens are only partly translated: some texts stay in English.")
}

impl App {
    /// Switch the app's language (View → Language, the setup), with the
    /// notice when it is not English.
    pub(crate) fn set_language(&mut self, code: &str) {
        self.lang = chummer_core::lang::Language::load(&self.lang_dir, code);
        self.ux.notice = needs_notice(code, &self.ux.prefs.notice_seen).then(|| code.to_owned());
    }

    /// Remember that the notice for `code` was read.
    pub(crate) fn notice_seen(&mut self, code: &str) {
        if !self.ux.prefs.notice_seen.iter().any(|s| s == code) {
            self.ux.prefs.notice_seen.push(code.to_owned());
            if let Err(e) = self.ux.prefs.save() {
                self.status = Some((format!("Could not save the settings: {e}"), true));
            }
        }
    }

    pub(crate) fn notice_window(&mut self, ctx: &egui::Context) {
        let Some(code) = self.ux.notice.clone() else { return };
        if self.ux.setup.is_some() {
            // The setup's language step says the same.
            return;
        }
        let mut open = true;
        let mut ok = false;
        egui::Window::new(self.lang.tr("Translation incomplete"))
            .id(egui::Id::new("translation_notice"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0])
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(notice_text(&self.lang));
                ui.add_space(6.0);
                if ui.button(self.lang.tr("Got it")).clicked() {
                    ok = true;
                }
            });
        if !open || ok {
            self.ux.notice = None;
            self.notice_seen(&code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_per_language() {
        assert!(!needs_notice("en-us", &[]));
        assert!(needs_notice("de-de", &[]));
        assert!(!needs_notice("de-de", &["fr-fr".into(), "de-de".into()]));
        assert!(needs_notice("ja-jp", &["de-de".into()]));
    }
}
