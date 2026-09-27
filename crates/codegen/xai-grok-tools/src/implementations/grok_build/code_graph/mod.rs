// The MCP client is the first non-test caller. These stay crate-private until then.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

mod limits;
mod select;

pub(crate) use limits::cap_text;
pub(crate) use select::{IndexedProject, parse_projects, select_project};
