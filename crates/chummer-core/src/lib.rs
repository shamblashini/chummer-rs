//! Core engine for chummer-rs, a Rust rewrite of Chummer5a.
//!
//! This crate has no UI dependencies. It loads Chummer's XML game data,
//! reads and writes `.chum5` characters, and computes the rules math.

pub mod xml;
pub mod data;
pub mod lang;
