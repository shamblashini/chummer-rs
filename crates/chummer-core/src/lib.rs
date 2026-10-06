//! Core engine for chummer-rs, a Rust rewrite of Chummer5a.
//!
//! This crate has no UI dependencies. It loads Chummer's XML game data,
//! reads and writes `.chum5` characters, and computes the rules math.

pub mod xml;
pub mod data;
pub mod custom_data;
pub mod lang;
pub mod expr;
pub mod settings;
pub mod dice;
pub mod improvement;
pub mod custom_improvement;
pub mod sections;
pub mod tree;
pub mod attributes;
pub mod character;
pub mod chum5lz;
pub mod html_color;
pub mod contacts;
pub mod skills;
pub mod calc;
pub mod engine;
pub mod format;
pub mod sources;
pub mod bonus;
pub mod requirements;
pub mod items;
pub mod chargen;
pub mod career;
pub mod essence_loss;
pub mod gm;
pub mod print;
pub mod export;
pub mod roster;
pub mod calendar;
pub mod play;
pub mod command;
pub mod campaign;
