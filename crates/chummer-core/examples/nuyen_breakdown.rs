//! Print what each owned item costs, by kind (debugging aid).
use chummer_core::character::Character;
use chummer_core::engine::Engine;
use chummer_core::items::{armor, cyberware, drug, gear, lifestyle, vehicle, weapon};

fn main() {
    let engine = Engine::load().unwrap();
    let path = std::env::args().nth(1).expect("file.chum5");
    let ch = Character::load(std::path::Path::new(&path)).unwrap();
    let store = engine.store_for_character(&ch);
    for (c, i) in [("gears", "gear"), ("cyberwares", "cyberware"), ("armors", "armor"), ("weapons", "weapon"), ("vehicles", "vehicle"), ("drugs", "drug"), ("lifestyles", "lifestyle")] {
        let mut total = 0.0;
        for e in ch.items(c, i) {
            let v = match i {
                "gear" => gear::cost(e),
                "cyberware" => cyberware::cost(&ch, &store, e),
                "armor" => armor::cost(e),
                "weapon" => weapon::cost(e),
                "vehicle" => vehicle::cost_with(&ch, &store, e),
                "drug" => drug::cost(e),
                _ => lifestyle::total_cost(&ch, e),
            };
            total += v;
            if v > std::env::var("MIN").ok().and_then(|s| s.parse().ok()).unwrap_or(5000.0) {
                println!("  {i:<10} {:<40} {v}", e.get("name"));
            }
        }
        println!("{c}: {total}");
    }
}

