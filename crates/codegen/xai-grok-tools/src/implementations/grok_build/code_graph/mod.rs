mod client;
mod limits;
mod select;
// Measurement scaffolding for the token-cost claim in the handover; `report()` has no production
// caller, so it must not compile into the shipped library. Reproduce with
// `cargo test -p xai-grok-tools code_graph::token_report -- --nocapture`.
#[cfg(test)]
mod token_report;
pub mod tools;

pub(crate) use limits::cap_text;
#[cfg(test)]
pub(crate) use select::IndexedProject;
pub(crate) use select::{parse_projects, select_project};
pub use tools::{BlastRadiusTool, SearchSymbolsTool, TraceCallsTool};
