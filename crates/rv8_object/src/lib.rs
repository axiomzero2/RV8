// CEP:FILE: crates/rv8_object/src/lib.rs
// CEP:WHAT: rv8_object: the universal object model, the deterministic ELF64 relocatable
//   and executable writers, the static linker with section GC, and honest stubs for
//   DWARF/Mach-O/COFF.
// CEP:WHY: STANDARDS 5.5: one format-agnostic model feeds every serializer and the
//   linker; two runs over the same input emit identical bytes, and the acceptance test
//   links and runs a real program that exits with code 13.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete (model, error, elf, link; dwarf/macho/coff remain honest stubs)
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: the library never panics; every fallible operation returns
//   Result<_, ObjectError> and all emission is bounds-checked against named budgets.
// CEP:ASSUMES: little-endian x86-64 ELF64 for the implemented writers; the model struct
//   shapes are a cross-agent contract with rv8_backend and must not change.
// CEP:COST: see per-module headers (emission is O(input bytes), GC is O(S + R)).
// CEP:EVIDENCE: crates/rv8_object/tests/acceptance.rs - end-to-end link-and-run exiting 13,
//   byte-determinism assertions, negative tests, GC behavior, and string-table suffix
//   sharing; unit tests in every module.
// CEP:SECURITY: untrusted object data is bounded by named MAX_* budgets and validated
//   before any writer touches it; no panics, no recursion over input structure.

pub mod error;
pub mod model;
pub mod elf;
pub mod macho;
pub mod coff;
pub mod link;
pub mod dwarf;

pub use error::ObjectError;
