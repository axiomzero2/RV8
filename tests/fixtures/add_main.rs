// CEP:FILE: tests/fixtures/add_main.rs
// CEP:WHAT: Minimal RV8 end-to-end fixture computing 6 + 7 in main and returning the result.
// CEP:WHY: Exit code 13 validates the full pipeline: parse, lower, select, allocate, emit, run.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: trivial.
// CEP:EVIDENCE: CI end-to-end step.

fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() -> i32 {
    add(6, 7)
}
