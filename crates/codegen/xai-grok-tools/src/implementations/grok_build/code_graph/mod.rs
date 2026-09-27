// Tools in the next task are the first non-test caller. Until then the client
// and these helpers are only reached from tests, so `cargo check --lib` would
// warn through the whole module.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

mod client;
mod limits;
mod select;

pub(crate) use limits::cap_text;
pub(crate) use select::{IndexedProject, parse_projects, select_project};
