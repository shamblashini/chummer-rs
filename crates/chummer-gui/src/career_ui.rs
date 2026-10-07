//! Career-mode actions shown above the karma and nuyen log: Edge spent
//! and regained, burning Edge and street cred, and magical group
//! membership. The rules are in `chummer_core::career::actions`.

use chummer_core::calc;
use chummer_core::career;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use eframe::egui;

use crate::doc::Doc;

/// Draw the row. Returns true if the character changed.
pub fn actions_ui(ui: &mut egui::Ui, ch: &mut Doc, engine: &Engine, lang: &Language) -> bool {
    let msg_id = egui::Id::new("career_actions_msg");
    let confirm_id = egui::Id::new("career_burn_edge_confirm");
    let mut result: Option<Command> = None;
    let rules = career::CareerRules::for_character(engine, ch).rules;
    let edge = calc::attribute_values(ch, "EDG", &rules);
    let used = ch.doc.get_i32("edgeused").unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} {}/{}", lang.tr("Edge"), (edge.total - used).max(0), edge.total));
        if ui.add_enabled(used < edge.total, egui::Button::new("−")).on_hover_text(lang.tr("Spend Edge")).clicked() {
            result = Some(Command::SpendEdge);
        }
        if ui.add_enabled(used > 0, egui::Button::new("+")).on_hover_text(lang.tr("Regain Edge")).clicked() {
            result = Some(Command::RegainEdge);
        }
        let confirming = ui.data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
        if confirming {
            ui.label(lang.tr("Are you sure you want to Burn this point of Edge?"));
            if ui.button(lang.tr("Burn a point of Edge")).clicked() {
                result = Some(Command::BurnEdge);
                ui.data_mut(|d| d.insert_temp(confirm_id, false));
            }
            if ui.button(lang.tr("Cancel")).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id, false));
            }
        } else if ui.add_enabled(edge.value > 0, egui::Button::new(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🔥")), lang.tr("Burn a point of Edge")))).clicked() {
            ui.data_mut(|d| d.insert_temp(confirm_id, true));
        }
        ui.separator();
        let rep = career::reputation_for(engine, ch);
        if ui.add_enabled(rep.street_cred >= 2, egui::Button::new(lang.tr("Burn Street Cred"))).on_hover_text(lang.tr_fmt("Burnt Street Cred: {0}", &[&rep.burnt_street_cred])).clicked() {
            result = Some(Command::BurnStreetCred);
        }
        if ch.mag_enabled() || ch.res_enabled() {
            ui.separator();
            let member = ch.flag("groupmember");
            let cost = career::group_karma_cost(engine, ch, !member);
            let mut on = member;
            let label = if cost > 0 { format!("{} ({})", lang.tr("Join Group"), lang.tr_fmt("{0} karma", &[&cost])) } else { lang.tr("Join Group") };
            if ui.checkbox(&mut on, label).changed() {
                result = Some(Command::SetGroupMember { member: on });
            }
        }
    });
    let changed = match result.map(|c| ch.apply(c)) {
        Some(Ok(_)) => {
            ui.data_mut(|d| d.remove::<String>(msg_id));
            true
        }
        Some(Err(e)) => {
            ui.data_mut(|d| d.insert_temp(msg_id, e.to_string()));
            false
        }
        None => false,
    };
    if let Some(msg) = ui.data(|d| d.get_temp::<String>(msg_id)) {
        ui.colored_label(ui.visuals().error_fg_color, msg);
    }
    changed
}
