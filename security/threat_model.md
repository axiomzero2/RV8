# Threat Model

## Abstract
This document defines the trust boundaries of the RV8 compiler, the inputs each boundary accepts, and the failure policy per boundary. It exists because standard sections 22.2 and 38.13 require a written threat model before any input-validation code is trusted.

## Formal Definitions
A **trust boundary** is a point where data produced by one party enters a component owned by another party. Inputs are classified **untrusted** by default. A component is **contained** if its failure cannot produce memory unsafety, unbounded resource use, or silent miscompilation.

## Algorithm
1. Classify every compiler input as untrusted.
2. Bound every recursive or size-driven structure at a named constant before descent.
3. Validate structure before interpretation; never interpret while validating.
4. Fail closed: invalid input produces a diagnostic and a nonzero exit, never a partial artifact.

## Invariants
1. Source text, macros, profiles, IR bitcode, object files, and linker inputs are untrusted until validated.
2. Every loop over untrusted input has a compile-time-derived or named bound.
3. No compiler diagnostic contains addresses, seeds, environment values, or file-system paths outside the session file list.
4. No input read path mutates compiler state before validation completes.

## Integration Points
- **Depends on:** `rv8_core::diag` (bounded diagnostics), `rv8_core::queue` (bounded queues).
- **Depended on by:** all crates; every parser and binary reader is a boundary implementation.

## Known Limitations
Proc-macro execution is not yet implemented; when it lands, it must run in a separate process with resource limits per standard section 22.11 before any untrusted crate may invoke it. The compiler currently accepts only its documented Rust subset; full Rust surface expands this model per feature.
