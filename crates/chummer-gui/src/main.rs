mod browser;
mod dice_ui;

use chummer_core::data::{self, DataStore};
use chummer_core::lang::Language;
use eframe::egui;

struct App {
    store: DataStore,
    lang: Language,
    browser: browser::DataBrowser,
    dice: dice_ui::DiceRoller,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::Window::new("Dice").show(ctx, |ui| self.dice.ui(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.browser.ui(ui, &self.store, &self.lang));
    }
}

fn main() -> eframe::Result {
    let store = DataStore::discover().expect("data");
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    eframe::run_native("Chummer", eframe::NativeOptions::default(), Box::new(|_cc| {
        Ok(Box::new(App { store, lang, browser: Default::default(), dice: Default::default() }))
    }))
}
