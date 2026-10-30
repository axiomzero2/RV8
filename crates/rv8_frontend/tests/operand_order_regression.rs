// CEP:FILE: crates/rv8_frontend/tests/operand_order_regression.rs
// CEP:WHAT: Twenty regression tests for the expression-flattener operand order
//   (RV8-137 follow-up): the LIFO work-stack delivered Binary operands and Call
//   arguments in reverse, so every non-commutative operator and call evaluated
//   backwards. Each test compiles source, flattens it, and pins the SoN input order.
// CEP:WHY: The defect inverted every comparison and subtraction while all existing
//   tests used commutative shapes; the batch makes the order contract load-bearing.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (test-only).
// CEP:ASSUMES: none
// CEP:COST: test-only.
// CEP:EVIDENCE: this module.
use rv8_frontend::compile_source;
use rv8_ir::son::{NodeId, Opcode};

/// CEP:WHAT: Regression 01_sub_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_01_sub_order() {
    let sea = lower("fn main() -> i32 { 10 - 3 }");
    let inputs = find_bin(&sea, Opcode::Sub);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(10), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(3), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 02_div_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_02_div_order() {
    let sea = lower("fn main() -> i32 { 20 / 4 }");
    let inputs = find_bin(&sea, Opcode::Div);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(20), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(4), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 03_rem_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_03_rem_order() {
    let sea = lower("fn main() -> i32 { 7 % 3 }");
    let inputs = find_bin(&sea, Opcode::Rem);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(7), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(3), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 04_shl_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_04_shl_order() {
    let sea = lower("fn main() -> i32 { 1 << 2 }");
    let inputs = find_bin(&sea, Opcode::Shl);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(1), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 05_shr_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_05_shr_order() {
    let sea = lower("fn main() -> i32 { 8 >> 1 }");
    let inputs = find_bin(&sea, Opcode::Shr);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(8), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(1), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 06_lt_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_06_lt_order() {
    let sea = lower("fn main() -> bool { 2 < 9 }");
    let inputs = find_bin(&sea, Opcode::CmpLt);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(2), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(9), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 07_le_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_07_le_order() {
    let sea = lower("fn main() -> bool { 5 <= 9 }");
    let inputs = find_bin(&sea, Opcode::CmpLe);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(5), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(9), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 08_eq_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_08_eq_order() {
    let sea = lower("fn main() -> bool { 4 == 4 }");
    let inputs = find_bin(&sea, Opcode::CmpEq);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(4), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(4), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 09_sub_const_right: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_09_sub_const_right() {
    let sea = lower("fn main() -> i32 { 9 - 2 }");
    let inputs = find_bin(&sea, Opcode::Sub);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(9), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 10_nested_left_assoc: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_10_nested_left_assoc() {
    let sea = lower("fn main() -> i32 { 10 - 4 - 2 }");
    let inputs = find_bin_outer(&sea, Opcode::Sub, true);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert!(matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Sub), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 11_nested_right: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_11_nested_right() {
    let sea = lower("fn main() -> i32 { 10 - (4 - 2) }");
    let inputs = find_bin_outer(&sea, Opcode::Sub, true);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(10), "operand 0 is the lhs");
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Sub), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 12_mixed_sub_div: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_12_mixed_sub_div() {
    let sea = lower("fn main() -> i32 { 12 / 3 / 2 }");
    let inputs = find_bin_outer(&sea, Opcode::Div, true);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert!(matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Div), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 13_unary_of_binary: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_13_unary_of_binary() {
    let sea = lower("fn main() -> i32 { 0 - (5 - 2) }");
    let inputs = find_bin(&sea, Opcode::Sub);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(5), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 14_add_commutes_but_order_pinned: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_14_add_commutes_but_order_pinned() {
    let sea = lower("fn main() -> i32 { 3 + 4 }");
    let inputs = find_bin(&sea, Opcode::Add);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(3), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(4), "operand 1 is the rhs");
}

/// CEP:WHAT: Regression 15_mul_order: the flattener delivers operands in source order.
/// CEP:WHY: The LIFO push-order defect delivered operands reversed, breaking every
/// non-commutative operator; this test pins the corrected order at the SoN level.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_15_mul_order() {
    let sea = lower("fn main() -> i32 { 6 * 7 }");
    let inputs = find_bin(&sea, Opcode::Mul);
    assert_eq!(inputs.len(), 2, "two data inputs");
    assert_eq!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(6), "operand 0 is the lhs");
    assert_eq!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(7), "operand 1 is the rhs");
}
/// CEP:WHAT: Regression 16: the loop guard keeps its source order (the RV8-137
/// end-to-end shape: while i < 6 lowers to CmpLt(i, 6), not CmpLt(6, i)).
/// CEP:WHY: The swapped comparison inverted every loop, computing the wrong sums.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_16_while_cond_order() {
    let sea = lower(
        "fn main() -> i32 { let mut i = 1; while i < 6 { i = i + 1; } i }",
    );
    let inputs = find_cmp(&sea, "CmpLt");
    assert_eq!(inputs.len(), 2);
    assert!(
        !matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(6)),
        "the loop variable is operand 0, not the constant"
    );
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(6)));
}

/// CEP:WHAT: Regression 17: an if condition keeps its source order.
/// CEP:WHY: Same LIFO defect family; branches inverted for every if.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_17_if_cond_order() {
    let sea = lower(
        "fn main() -> i32 { let x = 3; if x < 9 { x } else { 0 } }",
    );
    let inputs = find_cmp(&sea, "CmpLt");
    assert_eq!(inputs.len(), 2);
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(9)));
}

/// CEP:WHAT: Regression 18: call arguments arrive in declaration order.
/// CEP:WHY: The LIFO defect reversed argument order for every call.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_18_call_args_order() {
    let sea = lower(
        "fn f(a: i32, b: i32) -> i32 { a - b } fn main() -> i32 { f(8, 3) }",
    );
    let inputs = find_call(&sea);
    assert_eq!(inputs.len(), 2, "two call arguments");
    assert!(matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(8)));
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(3)));
}

/// CEP:WHAT: Regression 19: three call arguments arrive in declaration order.
/// CEP:WHY: Argument order for arity greater than two.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_19_call_three_args() {
    let sea = lower(
        "fn f(a: i32, b: i32, c: i32) -> i32 { a - b - c } \
         fn main() -> i32 { f(9, 4, 2) }",
    );
    let inputs = find_call(&sea);
    assert_eq!(inputs.len(), 3);
    assert!(matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Const(9)));
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(4)));
    assert!(matches!(sea.nodes[inputs[2].0 as usize].opcode, Opcode::Const(2)));
}

/// CEP:WHAT: Regression 20: a call nested in an argument keeps both orders.
/// CEP:WHY: Composition of the two corrected paths.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_order_20_nested_call_args() {
    let sea = lower(
        "fn f(a: i32, b: i32) -> i32 { a - b } \
         fn main() -> i32 { f(f(8, 3), 2) }",
    );
    let inputs = find_call(&sea);
    assert_eq!(inputs.len(), 2);
    assert!(
        matches!(sea.nodes[inputs[0].0 as usize].opcode, Opcode::Call { .. }),
        "the inner call is argument 0"
    );
    assert!(matches!(sea.nodes[inputs[1].0 as usize].opcode, Opcode::Const(2)));
}

// CEP:FILE-NOTE: helpers below are shared by every regression in this file.

/// CEP:WHAT: Compiles source text and flattens the first function to the Sea of Nodes.
/// CEP:WHY: Every regression asserts on the raw flattener output.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: src parses and type-checks.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn lower(src: &str) -> rv8_ir::son::SeaOfNodes {
    let hirs = compile_source(src).expect("frontend");
    let main = hirs.last().expect("at least one function");
    rv8_ir::lowering::hir_to_son(main).expect("flatten")
}

/// CEP:WHAT: Finds the first node with the given binary opcode and returns its inputs.
/// CEP:WHY: Assertion helper.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics when absent (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn find_bin(sea: &rv8_ir::son::SeaOfNodes, op: Opcode) -> Vec<NodeId> {
    find_bin_outer(sea, op, false)
}

/// CEP:WHAT: Finds a binary opcode node, outermost (last) or innermost (first).
/// CEP:WHY: Nested-shape regressions target the outer operator.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics when absent (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn find_bin_outer(
    sea: &rv8_ir::son::SeaOfNodes,
    op: Opcode,
    outer: bool,
) -> Vec<NodeId> {
    let mut hit: Option<&rv8_ir::son::SeaNode> = None;
    for n in &sea.nodes {
        if n.opcode == op {
            hit = match hit {
                None => Some(n),
                Some(_) if outer => Some(n),
                other => other,
            };
        }
    }
    match hit {
        Some(n) => n.inputs.clone(),
        None => panic!("binary node {op:?} not found"),
    }
}

/// CEP:WHAT: Finds the first comparison node of the named kind.
/// CEP:WHY: Assertion helper for conditions.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics when absent (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn find_cmp(sea: &rv8_ir::son::SeaOfNodes, kind: &str) -> Vec<NodeId> {
    for n in &sea.nodes {
        let hit = match kind {
            "CmpLt" => matches!(n.opcode, Opcode::CmpLt),
            "CmpLe" => matches!(n.opcode, Opcode::CmpLe),
            _ => matches!(n.opcode, Opcode::CmpEq),
        };
        if hit {
            return n.inputs.clone();
        }
    }
    panic!("comparison {kind} not found");
}

/// CEP:WHAT: Finds the first call node.
/// CEP:WHY: Assertion helper for argument order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics when absent (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn find_call(sea: &rv8_ir::son::SeaOfNodes) -> Vec<NodeId> {
    let mut hit: Option<&rv8_ir::son::SeaNode> = None;
    for n in &sea.nodes {
        if matches!(n.opcode, Opcode::Call { .. }) {
            hit = Some(n); // the outermost call is the last in topological order
        }
    }
    match hit {
        Some(n) => n.inputs.clone(),
        None => panic!("call node not found"),
    }
}
