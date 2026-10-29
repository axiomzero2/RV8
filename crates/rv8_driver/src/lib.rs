// CEP:FILE: crates/rv8_driver/src/lib.rs
// CEP:WHAT: rv8_driver, the compiler driver library: immutable session, fail-closed
//   error model, content addressing, stage-hash evidence, and the full pipeline wiring.
// CEP:WHY: The driver orchestrates every crate; module boundaries keep orchestration
//   auditable and the binary thin (main.rs holds only exit-code policy).
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold; library code only.
// CEP:EVIDENCE: workspace build plus the CI end-to-end and reproducibility gates.

pub mod cache;
pub mod error;
pub mod pipeline;
pub mod query;
pub mod session;
