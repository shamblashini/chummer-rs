//! Searchable drop-down lists.
//!
//! [`Combo`] is a drop-in for `egui::ComboBox`: a list with more than a
//! handful of entries gets a search field at the top that has focus when the
//! list opens, so typing filters it right away and Enter picks the first
//! match. Entries go through [`selectable_value`] and [`selectable_label`],
//! which hide what doesn't match while a list is being filtered and behave
//! exactly like `Ui::selectable_value` / `Ui::selectable_label` elsewhere.

use std::cell::RefCell;
use std::hash::Hash;

use eframe::egui::{
    self, response::Flags, Atoms, Button, Id, InnerResponse, IntoAtoms, Key, PopupCloseBehavior, Response, Sense, Ui, Widget,
    WidgetText,
};

/// Lists with more entries than this get a search field.
const SEARCH_FROM: usize = 8;

/// The filter of the list being drawn right now.
struct Scope {
    needle: String,
    /// Enter was pressed in the search field: the first match is picked.
    pick_first: bool,
    total: usize,
    shown: usize,
}

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// What a list remembers between frames.
#[derive(Clone, Default)]
struct State {
    text: String,
    /// Entries counted the last time the list was open.
    total: usize,
    /// Focus was given to the search field since the list opened.
    focused: bool,
}

pub struct Combo {
    inner: egui::ComboBox,
    salt: Id,
}

impl Combo {
    pub fn from_id_salt(salt: impl Hash) -> Self {
        let salt = Id::new(salt);
        Combo { inner: egui::ComboBox::from_id_salt(salt), salt }
    }

    pub fn selected_text(self, text: impl Into<WidgetText>) -> Self {
        Combo { inner: self.inner.selected_text(text), ..self }
    }

    pub fn width(self, width: f32) -> Self {
        Combo { inner: self.inner.width(width), ..self }
    }

    pub fn height(self, height: f32) -> Self {
        Combo { inner: self.inner.height(height), ..self }
    }

    pub fn show_ui<R>(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<Option<R>> {
        let key = ui.make_persistent_id(self.salt).with("search");
        let mut state: State = ui.data(|d| d.get_temp(key)).unwrap_or_default();
        // Clicks inside the list must not close it (the search field);
        // picking an entry closes it instead.
        let r = self.inner.close_behavior(PopupCloseBehavior::CloseOnClickOutside).show_ui(ui, |ui| {
            let mut pick_first = false;
            if state.total > SEARCH_FROM || !state.text.is_empty() {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.text).hint_text("🔍").desired_width(f32::INFINITY),
                );
                if !state.focused {
                    field.request_focus();
                    state.focused = true;
                }
                pick_first = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.separator();
            }
            let scope = Scope { needle: state.text.trim().to_lowercase(), pick_first, total: 0, shown: 0 };
            let outer = SCOPE.with(|s| s.replace(Some(scope)));
            let out = contents(ui);
            let scope = SCOPE.with(|s| s.replace(outer)).expect("combo scope");
            if scope.total > 0 && scope.shown == 0 {
                ui.weak("—");
            }
            state.total = scope.total;
            out
        });
        if r.inner.is_none() {
            state.text.clear();
            state.focused = false;
        }
        ui.data_mut(|d| d.insert_temp(key, state));
        r
    }
}

/// `Ui::selectable_label` that takes part in the surrounding [`Combo`]'s search.
pub fn selectable_label<'a>(ui: &mut Ui, checked: bool, text: impl IntoAtoms<'a>) -> Response {
    let atoms = Atoms::new(text);
    let first = match SCOPE.with(|s| filter(s.borrow_mut().as_mut(), &atoms)) {
        Visible::Hidden => return ui.interact(egui::Rect::NOTHING, ui.next_auto_id(), Sense::hover()),
        Visible::Shown { first } => first,
    };
    let mut response = Button::selectable(checked, atoms).ui(ui);
    let picked_by_enter = first && SCOPE.with(|s| s.borrow().as_ref().is_some_and(|s| s.pick_first));
    if picked_by_enter {
        response.flags.insert(Flags::FAKE_PRIMARY_CLICKED);
    }
    if response.clicked() && SCOPE.with(|s| s.borrow().is_some()) {
        ui.close();
    }
    response
}

/// `Ui::selectable_value` that takes part in the surrounding [`Combo`]'s search.
pub fn selectable_value<'a, V: PartialEq>(ui: &mut Ui, current: &mut V, value: V, text: impl IntoAtoms<'a>) -> Response {
    let mut response = selectable_label(ui, *current == value, text);
    if response.clicked() && *current != value {
        *current = value;
        response.mark_changed();
    }
    response
}

enum Visible {
    Hidden,
    Shown { first: bool },
}

fn filter(scope: Option<&mut Scope>, atoms: &Atoms) -> Visible {
    let Some(scope) = scope else { return Visible::Shown { first: false } };
    scope.total += 1;
    if !scope.needle.is_empty() {
        let text = atoms.text().unwrap_or_default().to_lowercase();
        if !matches(&text, &scope.needle) {
            return Visible::Hidden;
        }
    }
    scope.shown += 1;
    Visible::Shown { first: scope.shown == 1 }
}

/// Every word of the search appears in the entry, in any order.
pub(crate) fn matches(text: &str, needle: &str) -> bool {
    needle.split_whitespace().all(|w| text.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_in_any_order() {
        assert!(matches("ares predator v", "pred ares"));
        assert!(matches("ares predator v", ""));
        assert!(!matches("ares predator v", "colt"));
    }

    #[test]
    fn counts_and_hides() {
        let mut scope = Scope { needle: "pre".into(), pick_first: false, total: 0, shown: 0 };
        assert!(matches!(filter(Some(&mut scope), &Atoms::new("Colt America")), Visible::Hidden));
        assert!(matches!(filter(Some(&mut scope), &Atoms::new("Ares Predator")), Visible::Shown { first: true }));
        assert!(matches!(filter(Some(&mut scope), &Atoms::new("Predator II")), Visible::Shown { first: false }));
        assert_eq!((scope.total, scope.shown), (3, 2));
        assert!(matches!(filter(None, &Atoms::new("x")), Visible::Shown { first: false }));
    }

    /// Drives a real list headlessly: open it, type, press Enter.
    #[test]
    fn open_type_enter() {
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
        let ctx = egui::Context::default();
        let names: Vec<String> = (0..20).map(|i| format!("Item {i}")).collect();
        let mut current = String::new();
        let mut button = egui::Rect::NOTHING;
        let frame = |events: Vec<Event>, current: &mut String, button: &mut egui::Rect| -> Vec<String> {
            let mut shown = Vec::new();
            let input = RawInput { events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = Combo::from_id_salt("t").selected_text(current.clone()).show_ui(ui, |ui| {
                        for n in &names {
                            if selectable_value(ui, current, n.clone(), n).rect.is_positive() {
                                shown.push(n.clone());
                            }
                        }
                    });
                    *button = r.response.rect;
                });
            });
            shown
        };
        frame(vec![], &mut current, &mut button);
        let c = button.center();
        let click = |pressed| Event::PointerButton { pos: c, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(vec![Event::PointerMoved(c), click(true)], &mut current, &mut button);
        frame(vec![click(false)], &mut current, &mut button);
        // The first open frame counts the entries; the search field appears next.
        assert_eq!(frame(vec![], &mut current, &mut button).len(), 20);
        frame(vec![], &mut current, &mut button);
        let shown = frame(vec![Event::Text("item 1".into())], &mut current, &mut button);
        let shown = if shown.len() == 20 { frame(vec![], &mut current, &mut button) } else { shown };
        assert_eq!(shown.first().map(String::as_str), Some("Item 1"));
        assert_eq!(shown.len(), 11, "Item 1 and Item 10-19: {shown:?}");
        let enter = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        frame(vec![enter(true), enter(false)], &mut current, &mut button);
        frame(vec![], &mut current, &mut button);
        assert_eq!(current, "Item 1");
        assert!(frame(vec![], &mut current, &mut button).is_empty(), "list closed");
    }
}
