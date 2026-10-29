// CEP:FILE: crates/rv8_driver/src/lib.rs
// CEP:WHAT: Driver library modules for session, query system, cache, and pipeline wiring.
// CEP:WHY: The driver orchestrates all crates; module boundaries keep orchestration auditable.
// CEP:CLASS: HPC-1
// CEP:STATUS: stub
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: workspace build.

pub mod error;
pub mod session;
pub mod query;
pub mod cache;
pub mod pipeline;
