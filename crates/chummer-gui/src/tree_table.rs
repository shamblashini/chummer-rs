//! A tree table: Chummer5a's item tree views with our columns.
//!
//! Rows are the nodes of a [`chummer_core::tree::Node`] tree, flattened
//! with [`chummer_core::tree::flatten`]. The first column holds the
//! indentation, the guide lines and the ▸/▾ toggle; the other columns stay
//! aligned across depths. Group rows (Chummer's "Selected Gear", locations,
//! "Positive Qualities"...) only fill the first column and open or close on
//! a click. Item rows report clicks to the caller (to open the item editor)
//! and get a last column for per-row buttons.
//!
//! Which nodes are closed is kept in egui memory (saved with the window
//! state) per table, by node key (item guid or group key); everything
//! starts open, like Chummer's trees after a load. With a selected row,
//! Left / Right close and open it.

use std::collections::HashSet;
use std::hash::Hash;

use chummer_core::tree::{flatten, Node};
use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};
use egui_extras::{Column, TableBuilder};

use crate::theme::{self, ThemeKind};

/// What one row shows.
#[derive(Default)]
pub struct RowView {
    /// Cell texts, one per header. Group rows use only the first.
    pub cells: Vec<String>,
    /// A group row: no columns, bold, toggles on click.
    pub group: bool,
    /// Whether clicking the row selects it (items the editor handles).
    pub clickable: bool,
    /// Hover text for the first cell (item notes).
    pub hover: String,
    /// A creation problem with this item: (message, is an error), shown
    /// as a warning mark before the name.
    pub warning: Option<(String, bool)>,
}

/// What happened this frame.
#[derive(Default)]
pub struct TreeOutput {
    /// Key of the item row that was clicked.
    pub clicked: Option<String>,
}

pub struct TreeTable<'a> {
    id: egui::Id,
    headers: &'a [String],
    selected: Option<&'a str>,
}

/// Width of one indentation level.
const INDENT: f32 = 16.0;

impl<'a> TreeTable<'a> {
    pub fn new(id_salt: impl Hash, headers: &'a [String]) -> Self {
        TreeTable { id: egui::Id::new(("tree_table", id_salt)), headers, selected: None }
    }

    /// Key of the selected row, highlighted and moved by Left / Right.
    pub fn selected(mut self, key: Option<&'a str>) -> Self {
        self.selected = key;
        self
    }

    /// Draw the table. `view` describes a node's row; `actions` fills the
    /// last column of item rows (source link, remove button).
    pub fn show<T>(self, ui: &mut egui::Ui, roots: &[Node<T>], view: impl Fn(&Node<T>) -> RowView, mut actions: impl FnMut(&mut egui::Ui, &Node<T>)) -> TreeOutput {
        let closed_id = self.id.with("closed");
        let mut closed: HashSet<String> = ui.data_mut(|d| d.get_persisted::<HashSet<String>>(closed_id)).unwrap_or_default();
        let mut toggled: Option<String> = None;
        let mut out = TreeOutput::default();

        let rows = flatten(roots, &|n: &Node<T>| !closed.contains(&n.key));

        // Left / Right on the selected row, unless a text field has focus.
        let sel_row = self.selected.and_then(|sel| rows.iter().find(|r| r.node.key == sel));
        if let Some(r) = sel_row.filter(|r| !r.node.children.is_empty()) {
            let (left, right) = ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight)));
            if ((left && r.open) || (right && !r.open)) && !ui.ctx().wants_keyboard_input() {
                toggled = Some(r.node.key.clone());
            }
        }

        let th = theme::current(ui.ctx());
        let p = th.palette;
        let classic = th.kind == ThemeKind::Classic;
        let guide = if classic { p.weak } else { p.stroke };
        let row_h = ui.spacing().interact_size.y;
        let n_cols = self.headers.len().max(1);

        ui.push_id(self.id, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let mut tb = TableBuilder::new(ui)
                .id_salt("table")
                .striped(true)
                .resizable(true)
                .vscroll(false)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::auto().at_least(180.0));
            for _ in 1..n_cols {
                tb = tb.column(Column::auto().at_least(36.0));
            }
            tb = tb.column(Column::auto());
            tb.header(row_h + 2.0, |mut h| {
                for t in self.headers {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
                h.col(|_| {});
            })
            .body(|body| {
                body.rows(row_h, rows.len(), |mut row| {
                    let index = row.index();
                    let r = &rows[index];
                    let v = view(r.node);
                    let selected = self.selected == Some(r.node.key.as_str());
                    row.set_selected(selected);
                    let text_color = if selected { p.selection_text } else { p.text };
                    let has_children = !r.node.children.is_empty();
                    let mut toggle_hit = false;
                    row.col(|ui| {
                        let rect = ui.max_rect();
                        let x = |level: usize| rect.left() + level as f32 * INDENT + INDENT / 2.0;
                        let (top, mid, bottom) = (rect.top() - 1.0, rect.center().y, rect.bottom() + 1.0);
                        let painter = ui.painter();
                        let line = |a: egui::Pos2, b: egui::Pos2| {
                            if classic {
                                painter.extend(egui::Shape::dotted_line(&[a, b], guide, 2.0, 0.5));
                            } else {
                                painter.line_segment([a, b], Stroke::new(1.0_f32, guide));
                            }
                        };
                        let own = x(r.depth);
                        if classic {
                            // WinForms TreeView lines: siblings are joined
                            // below their parent's box (roots in column 0),
                            // with a stub to their own box.
                            let col = |depth: usize| x(depth.saturating_sub(1));
                            for (depth, &more) in r.guides.iter().enumerate() {
                                if more {
                                    line(egui::pos2(col(depth), top), egui::pos2(col(depth), bottom));
                                }
                            }
                            let from = if r.depth == 0 && index == 0 { mid } else { top };
                            line(egui::pos2(col(r.depth), from), egui::pos2(col(r.depth), if r.last { mid } else { bottom }));
                            line(egui::pos2(col(r.depth), mid), egui::pos2(own + INDENT / 2.0, mid));
                            if r.open {
                                line(egui::pos2(own, mid), egui::pos2(own, bottom));
                            }
                        } else {
                            // Indent guides under each open ancestor.
                            for level in 0..r.depth {
                                line(egui::pos2(x(level), top), egui::pos2(x(level), bottom));
                            }
                        }
                        let toggle_rect = egui::Rect::from_center_size(egui::pos2(own, mid), egui::vec2(INDENT, row_h));
                        if has_children {
                            let resp = ui.interact(toggle_rect, ui.id().with(("toggle", &r.node.key)), Sense::click());
                            let c = toggle_rect.center();
                            let hot = resp.hovered();
                            if classic {
                                // The WinForms [+] / [-] box.
                                let b = egui::Rect::from_center_size(c, egui::vec2(9.0, 9.0));
                                ui.painter().rect(b, 0.0, p.field, Stroke::new(1.0_f32, if hot { p.stroke_focus } else { p.weak }), egui::StrokeKind::Inside);
                                let ink = Stroke::new(1.0_f32, p.text);
                                ui.painter().line_segment([egui::pos2(c.x - 2.5, c.y), egui::pos2(c.x + 2.5, c.y)], ink);
                                if !r.open {
                                    ui.painter().line_segment([egui::pos2(c.x, c.y - 2.5), egui::pos2(c.x, c.y + 2.5)], ink);
                                }
                            } else {
                                // ▸ / ▾, drawn: the UI fonts have no such glyphs.
                                let color = if hot { p.accent } else if selected { p.selection_text } else { p.weak };
                                let pts = if r.open {
                                    vec![egui::pos2(c.x - 4.0, c.y - 2.0), egui::pos2(c.x + 4.0, c.y - 2.0), egui::pos2(c.x, c.y + 3.0)]
                                } else {
                                    vec![egui::pos2(c.x - 2.0, c.y - 4.0), egui::pos2(c.x + 3.0, c.y), egui::pos2(c.x - 2.0, c.y + 4.0)]
                                };
                                if hot {
                                    ui.painter().rect_filled(egui::Rect::from_center_size(c, egui::vec2(14.0, 14.0)), 3.0, p.surface_hover);
                                }
                                ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
                            }
                            if resp.clicked() {
                                toggle_hit = true;
                            }
                        }
                        ui.add_space((r.depth + 1) as f32 * INDENT + 2.0);
                        if let Some((msg, error)) = &v.warning {
                            theme::warning_mark(ui, *error).on_hover_text(msg);
                        }
                        let name = v.cells.first().cloned().unwrap_or_default();
                        let mut text = RichText::new(name).color(text_color);
                        if v.group {
                            text = text.strong();
                        }
                        let label = ui.add(egui::Label::new(text).selectable(false).sense(Sense::hover()));
                        if !v.hover.trim().is_empty() {
                            label.on_hover_text(&v.hover);
                        }
                    });
                    for i in 1..n_cols {
                        row.col(|ui| {
                            match v.cells.get(i) {
                                Some(c) if !v.group => {
                                    ui.add_space(2.0);
                                    ui.add(egui::Label::new(RichText::new(c).color(text_color)).selectable(false));
                                }
                                _ => {}
                            }
                        });
                    }
                    row.col(|ui| {
                        if !v.group {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if selected {
                                // Frameless icons (📖) must stay visible on the selection.
                                ui.visuals_mut().override_text_color = Some(p.selection_text);
                            }
                            actions(ui, r.node);
                        }
                    });
                    let resp = row.response();
                    if toggle_hit || (has_children && (v.group && resp.clicked() || resp.double_clicked())) {
                        toggled = Some(r.node.key.clone());
                    } else if resp.clicked() && v.clickable {
                        out.clicked = Some(r.node.key.clone());
                    }
                    if v.clickable {
                        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    }
                });
            });
        });

        if let Some(k) = toggled {
            if !closed.remove(&k) {
                closed.insert(k);
            }
            ui.data_mut(|d| d.insert_persisted(closed_id, closed));
            ui.ctx().request_repaint();
        }
        out
    }
}
