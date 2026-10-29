// CEP:FILE: crates/rv8_middle/src/error.rs
// CEP:WHAT: The rv8_middle fail-closed error contract: verifier violations, pass failures,
//   lifted IR errors, and named-budget exhaustion, unified in one enum with Display and
//   std::error::Error implementations.
// CEP:WHY: Standard 38.6 bans panics in library code; every rv8_middle entry point returns
//   Result<_, MiddleError> so untrusted IR can only fail closed with a named reason.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; this type is the failure contract for the crate.
// CEP:ASSUMES: violation vectors are produced by rv8_middle::verifier and are ordered by
//   node id for determinism.
// CEP:COST: allocation only on error paths.
// CEP:EVIDENCE: unit tests rv8_middle::error::tests::display_is_stable.
// CEP:SECURITY: no panics on any input; Display output contains node ids only, never
//   environment or address data.

use crate::verifier::VerifyViolation;
use rv8_ir::hetir::HetError;
use rv8_ir::son::NodeError;
use std::fmt;

/// CEP:WHAT: Error returned by every fallible rv8_middle entry point.
/// CEP:WHY: One error surface keeps pipeline glue total; variants name the stage and the
///   offending ids so diagnostics need no backtrace.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only for Verify vectors.
/// CEP:EVIDENCE: unit test rv8_middle::error::tests::display_is_stable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiddleError {
    /// Structural Sea-of-Nodes error lifted from an rv8_ir accessor or verify_structure.
    Ir(NodeError),
    /// Heterogeneous-IR error lifted from HetIr::verify during optimize_hetir.
    Het(HetError),
    /// One or more deep-verifier violations; the vector reports ALL violations found.
    Verify(Vec<VerifyViolation>),
    /// A named pass failed for a reason the pass documents.
    Pass { pass: &'static str, detail: &'static str },
    /// A named compile-time budget was exceeded (38.12); never OOM or panic.
    Budget { budget: &'static str, limit: u64, actual: u64 },
}

impl fmt::Display for MiddleError {
    /// CEP:WHAT: Renders the error in stable text form for diagnostics.
    /// CEP:WHY: Printable failure contract (38.15); deterministic text, node ids only.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(violations).
    /// CEP:EVIDENCE: unit test rv8_middle::error::tests::display_is_stable.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MiddleError::Ir(e) => write!(f, "sea-of-nodes structure error: {e:?}"),
            MiddleError::Het(e) => write!(f, "heterogeneous-ir error: {e:?}"),
            MiddleError::Verify(vs) => {
                write!(f, "verification failed with {} violation(s):", vs.len())?;
                for v in vs {
                    write!(f, " n{} {:?}", v.node.0, v.kind)?;
                }
                Ok(())
            }
            MiddleError::Pass { pass, detail } => {
                write!(f, "pass {pass} failed: {detail}")
            }
            MiddleError::Budget { budget, limit, actual } => {
                write!(f, "budget {budget} exceeded: limit {limit}, actual {actual}")
            }
        }
    }
}

impl std::error::Error for MiddleError {}

impl From<NodeError> for MiddleError {
    /// CEP:WHAT: Lifts a NodeError into MiddleError.
    /// CEP:WHY: One error type at the pipeline boundary (?-composition).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_middle::error::tests::display_is_stable.
    fn from(e: NodeError) -> MiddleError {
        MiddleError::Ir(e)
    }
}

impl From<HetError> for MiddleError {
    /// CEP:WHAT: Lifts a HetError into MiddleError.
    /// CEP:WHY: One error type at the pipeline boundary (?-composition).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_middle::error::tests::display_is_stable.
    fn from(e: HetError) -> MiddleError {
        MiddleError::Het(e)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies Display text is stable and total over every variant.
    /// CEP:WHY: Diagnostics must be deterministic (38.15) and never panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_is_stable() {
        let b = MiddleError::Budget { budget: "MAX_MIDDLE_NODES", limit: 10, actual: 11 };
        assert_eq!(b.to_string(), "budget MAX_MIDDLE_NODES exceeded: limit 10, actual 11");
        let p = MiddleError::Pass { pass: "GVN", detail: "unit" };
        assert_eq!(p.to_string(), "pass GVN failed: unit");
        let v = MiddleError::Verify(vec![crate::verifier::VerifyViolation {
            node: rv8_ir::son::NodeId(3),
            kind: crate::verifier::VerifyKind::PhiMisplaced,
        }]);
        assert!(v.to_string().contains("verification failed"));
        let lifted: MiddleError = rv8_ir::son::NodeError::DanglingNode(rv8_ir::son::NodeId(1)).into();
        assert!(lifted.to_string().starts_with("sea-of-nodes structure error"));
        let het: MiddleError = rv8_ir::hetir::HetError::DeadPureNode { node: rv8_ir::hetir::HetNodeId(2) }.into();
        assert!(het.to_string().starts_with("heterogeneous-ir error"));
    }
}
