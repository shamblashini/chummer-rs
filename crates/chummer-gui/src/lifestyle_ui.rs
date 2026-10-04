//! Editing a lifestyle the character has (what `SelectLifestyle` and
//! `SelectLifestyleAdvanced` change when reopened): months, roommates,
//! percentage paid, bought area/comforts/security points, and lifestyle
//! qualities, with the monthly cost shown live.

use chummer_core::career::{self, NuyenExpenseType};
use chummer_core::character::Character;
use chummer_core::data::{self, Record};
use chummer_core::format;
use chummer_core::items::lifestyle::{self, Options};
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::magic_ui::{Ctx, Pick, Picker};
use crate::pdf_ui::Status;

const STYLES: &[&str] = &["Standard", "Advanced", "BoltHole", "Safehouse"];

#[derive(Default)]
pub struct LifestyleEditor {
    /// Guid of the lifestyle being edited.
    selected: Option<String>,
    /// Quality picker for (lifestyle guid).
    picker: Option<(String, Picker)>,
    free_quality: bool,
}

impl LifestyleEditor {
    /// Returns true if the character changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let lifestyles: Vec<Element> = ch.items("lifestyles", "lifestyle").into_iter().cloned().collect();
        if lifestyles.is_empty() {
            return false;
        }
        if !self.selected.as_ref().is_some_and(|g| lifestyles.iter().any(|l| &l.get("guid") == g)) {
            self.selected = Some(lifestyles[0].get("guid"));
        }
        let guid = self.selected.clone().unwrap_or_default();
        let Some(l) = lifestyles.iter().find(|l| l.get("guid") == guid) else { return false };
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new("Edit lifestyle").strong()).id_salt("lifestyle_ed").default_open(true).show(ui, |ui| {
            if lifestyles.len() > 1 {
                egui::ComboBox::from_id_salt("lifestyle_pick").selected_text(l.get("name")).width(260.0).show_ui(ui, |ui| {
                    for x in &lifestyles {
                        let g = x.get("guid");
                        if ui.selectable_label(g == guid, x.get("name")).clicked() {
                            self.selected = Some(g);
                        }
                    }
                });
            }
            changed |= self.options_ui(ui, ch, l, status);
            ui.add_space(6.0);
            changed |= self.qualities_ui(ui, ch, cx, l, status);
        });
        changed
    }

    fn options_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, l: &Element, status: &mut Status) -> bool {
        let guid = l.get("guid");
        let before = Options::from_saved(l);
        let mut o = before.clone();
        let (max_area, max_comforts, max_security) = lifestyle::point_limits(l);
        let advanced = o.style != "Standard";
        let mut pay_month = false;
        egui::Grid::new(("lifestyle_opts", &guid)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(240.0));
            ui.end_row();
            ui.label("Base lifestyle");
            ui.label(l.get("baselifestyle"));
            ui.end_row();
            ui.label("Type");
            egui::ComboBox::from_id_salt(("lifestyle_type", &guid)).selected_text(o.style.clone()).show_ui(ui, |ui| {
                for s in STYLES {
                    ui.selectable_value(&mut o.style, (*s).to_owned(), *s);
                }
            });
            ui.end_row();
            let unit = match l.get("increment").as_str() {
                "Day" => "Days",
                "Week" => "Weeks",
                _ => "Months",
            };
            ui.label(unit);
            ui.horizontal(|ui| {
                if ch.created {
                    // Career: more months are paid for one at a time.
                    ui.label(o.months.to_string());
                    if ui.small_button("−").on_hover_text("No refund").clicked() && o.months > 1 {
                        o.months -= 1;
                    }
                    let cost = lifestyle::monthly_cost(ch, l);
                    if ui.add_enabled(ch.nuyen + 1e-9 >= cost, egui::Button::new(format!("Pay one more ({})", format::nuyen(cost)))).clicked() {
                        pay_month = true;
                    }
                } else {
                    ui.add(egui::DragValue::new(&mut o.months).range(1..=999));
                }
            });
            ui.end_row();
            ui.label("Roommates");
            ui.add_enabled(!o.trust_fund, egui::DragValue::new(&mut o.roommates).range(0..=20));
            ui.end_row();
            ui.label("Percentage paid");
            ui.add(egui::DragValue::new(&mut o.percentage).range(0.0..=100.0).suffix(" %"));
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut o.split_cost_with_roommates, "Split cost with roommates");
                ui.checkbox(&mut o.trust_fund, "Trust fund");
            });
            ui.end_row();
            for (label, v, max, base) in [
                ("Comforts", &mut o.comforts, max_comforts, l.get_i32("basecomforts").unwrap_or(0)),
                ("Neighborhood", &mut o.area, max_area, l.get_i32("basearea").unwrap_or(0)),
                ("Security", &mut o.security, max_security, l.get_i32("basesecurity").unwrap_or(0)),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    ui.add_enabled(advanced && max > 0, egui::DragValue::new(v).range(0..=max)).on_disabled_hover_text(if advanced { "At the base lifestyle's limit" } else { "Only advanced lifestyles buy points" });
                    ui.weak(format!("base {base}, up to +{max}"));
                });
                ui.end_row();
            }
            if l.get_bool("allowbonuslp").unwrap_or(false) {
                ui.label("Bonus LP");
                ui.add_enabled(advanced, egui::DragValue::new(&mut o.bonus_lp).range(0..=20));
                ui.end_row();
            }
            ui.label("Cost");
            let monthly = lifestyle::monthly_cost(ch, l);
            ui.strong(format!("{} per {} · {} total", format::nuyen(monthly), unit.trim_end_matches('s').to_lowercase(), format::nuyen(lifestyle::total_cost(ch, l))));
            ui.end_row();
        });
        let mut changed = false;
        if pay_month {
            let cost = lifestyle::monthly_cost(ch, l);
            match career::spend_nuyen(ch, cost, &format!("Lifestyle {}", l.get("name")), NuyenExpenseType::IncreaseLifestyle, &guid, 0.0) {
                Ok(_) => {
                    o.months += 1;
                    *status = Some((format!("Paid {} for {}", format::nuyen(cost), l.get("name")), false));
                }
                Err(e) => *status = Some((e.to_string(), true)),
            }
        }
        if !same(&o, &before) {
            changed |= lifestyle::update(ch, &guid, &o);
        }
        changed
    }

    fn qualities_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, l: &Element, status: &mut Status) -> bool {
        let guid = l.get("guid");
        let mut changed = false;
        let quals: Vec<Element> = l.child("lifestylequalities").map(|q| q.children_named("lifestylequality").cloned().collect()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.strong(format!("Lifestyle qualities ({})", quals.len()));
            if ui.button("➕ Add quality…").clicked() {
                self.picker = Some((guid.clone(), Picker::new("Add lifestyle quality", "lifestyles.xml", "qualities", "quality", cx.books())));
            }
            ui.checkbox(&mut self.free_quality, "Free");
        });
        if !quals.is_empty() {
            egui::Grid::new(("lifestyle_quals", &guid)).striped(true).num_columns(5).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in ["Quality", "Category", "LP", "Cost", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for q in &quals {
                    let extra = q.get("extra");
                    ui.label(if extra.is_empty() { q.get("name") } else { format!("{} ({extra})", q.get("name")) });
                    ui.label(q.get("category"));
                    ui.label(q.get("lp"));
                    let builtin = q.get("lifestylequalitysource") == "BuiltIn";
                    let free = builtin || q.get_bool("free").unwrap_or(false);
                    let mult = q.get_f64("multiplier").unwrap_or(0.0);
                    let cost = if free {
                        "free".to_owned()
                    } else if mult != 0.0 {
                        format!("{mult:+}%")
                    } else {
                        q.get("cost")
                    };
                    ui.label(cost);
                    if builtin {
                        ui.weak("built in");
                    } else if ui.small_button("🗑").on_hover_text("Remove").clicked() {
                        changed |= lifestyle::remove_quality(ch, &guid, &q.get("guid"));
                    }
                    ui.end_row();
                }
            });
        }
        if let Some((lguid, picker)) = self.picker.as_mut() {
            let store = cx.store;
            let chr: &Character = ch;
            let base = l.get("baselifestyle");
            let choices = |r: Record<'_>| lifestyle::quality_choices(chr, store, r);
            let note = |r: Record<'_>| {
                let mut parts = Vec::new();
                if !r.get("lp").is_empty() {
                    parts.push(format!("{} LP", r.get("lp")));
                }
                if !r.get("multiplier").is_empty() {
                    parts.push(format!("{}%", r.get("multiplier")));
                } else if !r.get("cost").is_empty() {
                    parts.push(format!("{}¥", r.get("cost")));
                }
                if r.get("allowed").split(',').any(|a| a == base) {
                    parts.push("free here".into());
                }
                parts.join(" · ")
            };
            match picker.show(ui.ctx(), store, None, &choices, &note) {
                Pick::None => {}
                Pick::Cancel => self.picker = None,
                Pick::Done(name, answer) => {
                    let lguid = lguid.clone();
                    let r = store.doc("lifestyles.xml").map_err(|e| e.to_string()).and_then(|doc| {
                        let rec = data::find(&doc, "qualities", "quality", &name).ok_or_else(|| format!("unknown quality {name}"))?;
                        lifestyle::add_quality(ch, store, &lguid, rec, answer.as_deref(), self.free_quality)
                    });
                    match r {
                        Ok(_) => {
                            *status = Some((format!("Added {name}"), false));
                            changed = true;
                        }
                        Err(e) => *status = Some((e, true)),
                    }
                    self.picker = None;
                }
            }
        }
        changed
    }
}

/// Options compare (no `PartialEq` on the core type).
fn same(a: &Options, b: &Options) -> bool {
    a.name == b.name
        && a.months == b.months
        && a.roommates == b.roommates
        && a.percentage == b.percentage
        && a.area == b.area
        && a.comforts == b.comforts
        && a.security == b.security
        && a.bonus_lp == b.bonus_lp
        && a.trust_fund == b.trust_fund
        && a.split_cost_with_roommates == b.split_cost_with_roommates
        && a.style == b.style
}
