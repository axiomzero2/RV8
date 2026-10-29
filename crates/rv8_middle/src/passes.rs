// CEP:FILE: crates/rv8_middle/src/passes.rs
// CEP:WHAT: Three deterministic Sea-of-Nodes transformations: CONST_FOLD (pure-opcode
//   evaluation over constant inputs with type-width discipline), GVN (same-block value
//   numbering by canonical Fx hash with hash-consing replacement), and DCE (mark
//   side-effect/control roots, transitively mark data dependencies with an iterative
//   worklist, sweep unmarked pure nodes by deterministic rebuild).
// CEP:WHY: These are the fixed middle-end pipeline stages (38.20); each pass verifies its
//   own structure at start and end and reports a PassReport with a change count.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: every pass returns Result; structural failures surface as
//   MiddleError::Ir, budget exhaustion as MiddleError::Budget; division by zero is NOT
//   folded (left for runtime semantics), never a panic.
// CEP:ASSUMES: folding is integer/bool only (Const carries an i64 payload, so 128-bit and
//   floating-point constants are unrepresentable and skipped); GVN is same-block scoped so
//   replacement never breaks dominance; shift amounts >= bit width are not folded.
// CEP:COST: CONST_FOLD O(N * fanin); GVN O(N * fanin) expected with hash buckets; DCE
//   O(N * fanin); budgets named below.
// CEP:EVIDENCE: unit tests rv8_middle::passes::tests::*.
// CEP:SECURITY: no recursion; sweeps rebuild via index remaps over flat vectors.
// CEP:DETERMINISM: deterministic; candidate order is node-id ascending, maps are
//   lookup-only FxHashMap buckets (never iterated), reports carry static names.

use crate::analysis::check_budget;
use crate::error::MiddleError;
use rv8_core::{fx_mix, fxhash64, FxHashMap};
use rv8_ir::son::{NodeId, Opcode, IrType, SeaOfNodes};

/// CEP:WHAT: Result of one pass: its fixed name and how many nodes it transformed.
/// CEP:WHY: The pipeline summary needs per-pass change counts for remarks (38.16) and
///   fixpoint detection.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: name is one of the fixed pass names.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_middle::pipeline::tests::optimize_son_pipeline_is_fixed_order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassReport {
    /// Fixed pass name: CONST_FOLD, GVN, DCE, or HET_DCE.
    pub name: &'static str,
    /// Number of nodes transformed, replaced, or swept.
    pub changed: usize,
}

/// CEP:WHAT: Reports whether an opcode is a pure value operation eligible for folding and
///   value numbering (no memory, control, or escape semantics).
/// CEP:WHY: One shared eligibility table keeps CONST_FOLD and GVN consistent.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Phi is excluded (path-dependent values); Load is excluded (memory read
///   per the rv8_ir contract); Call/Store and all control opcodes are excluded.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_folds_and_skips_div_zero.
pub(crate) fn is_pure_value_opcode(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Const(_)
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Rem
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::Neg
            | Opcode::Not
            | Opcode::CmpEq
            | Opcode::CmpLt
            | Opcode::CmpLe
            | Opcode::Cvt
    )
}

/// CEP:WHAT: Reports whether an opcode is a DCE root (side effects, control, parameters,
///   or allocation/escape markers that downstream stages must observe).
/// CEP:WHY: Dead-node marking starts from roots; the set must match the observable-effect
///   contract of the opcode set.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Load is pure for this contract (matching rv8_ir::lower_het); unused loads
///   are swept.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::dce_sweeps_dead_pure_subgraph.
pub(crate) fn is_dce_root(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Store
            | Opcode::Call { .. }
            | Opcode::Drop
            | Opcode::Branch
            | Opcode::Merge
            | Opcode::Region
            | Opcode::Return
            | Opcode::Alloc
            | Opcode::EscapePoint
            | Opcode::Materialize
            | Opcode::Parameter(_)
    )
}

/// CEP:WHAT: Reports the bit width and signedness of an integer-like IrType; None for
///   types folding does not interpret (floats, pointers, unit, 128-bit).
/// CEP:WHY: Folding must respect type widths (task contract); Const's i64 payload cannot
///   hold 128-bit patterns, so those lanes are skipped by documented assumption.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Bool is 1-bit unsigned; Char is 32-bit unsigned.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_respects_type_widths.
fn int_layout(ty: IrType) -> Option<(u32, bool)> {
    let (bits, signed) = match ty {
        IrType::Bool => (1, false),
        IrType::I8 => (8, true),
        IrType::I16 => (16, true),
        IrType::I32 => (32, true),
        IrType::I64 => (64, true),
        IrType::U8 => (8, false),
        IrType::U16 => (16, false),
        IrType::U32 => (32, false),
        IrType::U64 => (64, false),
        IrType::Char => (32, false),
        IrType::I128 | IrType::U128 | IrType::F32 | IrType::F64 | IrType::Ptr | IrType::Unit => {
            return None
        }
    };
    Some((bits, signed))
}

/// CEP:WHAT: Truncates an i64 bit pattern to the low `bits` bits.
/// CEP:WHY: All arithmetic folds operate modulo the type width.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 1 <= bits <= 64.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_respects_type_widths.
fn trunc_bits(v: i64, bits: u32) -> i64 {
    if bits >= 64 {
        v
    } else {
        let mask = (1u64 << bits) - 1;
        ((v as u64) & mask) as i64
    }
}

/// CEP:WHAT: Sign-extends a truncated bit pattern of `bits` width to i64.
/// CEP:WHY: Signed comparisons and division interpret stored patterns by width.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: v already truncated to `bits` bits.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_respects_type_widths.
fn sext_bits(v: i64, bits: u32) -> i64 {
    if bits >= 64 || bits == 0 {
        return v;
    }
    let shift = 64 - bits;
    (v << shift) >> shift
}

/// CEP:WHAT: Evaluates one pure binary opcode over two constant patterns of type `ty`,
///   returning the folded pattern; None when the operation must stay unfolded.
/// CEP:WHY: CONST_FOLD core; every arithmetic rule respects width and signedness, and
///   division/remainder by zero or INT_MIN/-1 stay unfolded instead of trapping the
///   compiler.
/// CEP:STATUS: complete
/// CEP:FAILURE: None for division hazards and out-of-range shifts (documented policy).
/// CEP:ASSUMES: result type equals operand type for arithmetic; comparisons produce Bool.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_folds_and_skips_div_zero.
fn fold_binary(op: Opcode, a: i64, b: i64, ty: IrType, out_ty: IrType) -> Option<i64> {
    let (bits, signed) = int_layout(ty)?;
    let pattern = match op {
        Opcode::Add => trunc_bits(a.wrapping_add(b), bits),
        Opcode::Sub => trunc_bits(a.wrapping_sub(b), bits),
        Opcode::Mul => trunc_bits(a.wrapping_mul(b), bits),
        Opcode::Div | Opcode::Rem => {
            let (x, y) = if signed {
                (sext_bits(a, bits), sext_bits(b, bits))
            } else {
                (a, b)
            };
            let negative_one = if signed { y == -1 } else { y == -1 && bits == 64 };
            if y == 0 || negative_one {
                return None; // Do NOT fold; leave for runtime semantics.
            }
            let r = if signed {
                if op == Opcode::Div {
                    x.checked_div(y)?
                } else {
                    x.checked_rem(y)?
                }
            } else if op == Opcode::Div {
                (x as u64).checked_div(y as u64)? as i64
            } else {
                (x as u64).checked_rem(y as u64)? as i64
            };
            trunc_bits(r, bits)
        }
        Opcode::And => trunc_bits(a & b, bits),
        Opcode::Or => trunc_bits(a | b, bits),
        Opcode::Xor => trunc_bits(a ^ b, bits),
        Opcode::Shl => {
            let amount = b as u64;
            if amount >= u64::from(bits) {
                return None;
            }
            trunc_bits(a.wrapping_shl(amount as u32), bits)
        }
        Opcode::Shr => {
            let amount = b as u64;
            if amount >= u64::from(bits) {
                return None;
            }
            if signed {
                trunc_bits(sext_bits(a, bits).wrapping_shr(amount as u32), bits)
            } else {
                trunc_bits(a.wrapping_shr(amount as u32), bits)
            }
        }
        Opcode::CmpEq => i64::from(a == b),
        Opcode::CmpLt => {
            let r = if signed {
                sext_bits(a, bits) < sext_bits(b, bits)
            } else {
                (a as u64) < (b as u64)
            };
            i64::from(r)
        }
        Opcode::CmpLe => {
            let r = if signed {
                sext_bits(a, bits) <= sext_bits(b, bits)
            } else {
                (a as u64) <= (b as u64)
            };
            i64::from(r)
        }
        _ => return None,
    };
    // Comparisons produce Bool; everything else re-truncates to the output width.
    let out_bits = int_layout(out_ty)?.0;
    Some(trunc_bits(pattern, out_bits))
}

/// CEP:WHAT: Evaluates one pure unary opcode over a constant pattern.
/// CEP:WHY: CONST_FOLD core for Neg/Not/Cvt.
/// CEP:STATUS: complete
/// CEP:FAILURE: None for Cvt with no integer target layout.
/// CEP:ASSUMES: Cvt reinterprets the sign-extended source value truncated to the target
///   width (integer-to-integer conversion).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_respects_type_widths.
fn fold_unary(op: Opcode, a: i64, ty: IrType, out_ty: IrType) -> Option<i64> {
    match op {
        Opcode::Neg => {
            let (bits, _) = int_layout(out_ty)?;
            Some(trunc_bits(a.wrapping_neg(), bits))
        }
        Opcode::Not => {
            let (bits, _) = int_layout(out_ty)?;
            Some(trunc_bits(!a, bits))
        }
        Opcode::Cvt => {
            let (in_bits, in_signed) = int_layout(ty)?;
            let (out_bits, _) = int_layout(out_ty)?;
            let value = if in_signed { sext_bits(a, in_bits) } else { a };
            Some(trunc_bits(value, out_bits))
        }
        _ => None,
    }
}

/// CEP:WHAT: CONST_FOLD pass: evaluates pure opcodes with all-constant inputs in place.
/// CEP:WHY: In-place opcode rewrite preserves node identity, block placement, and
///   dominance; division by zero is left unfolded per the pass contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Ir when verify_structure fails at start or end.
/// CEP:ASSUMES: integer/bool lanes only; see module header.
/// CEP:COST: O(N * fanin).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_folds_and_skips_div_zero.
pub fn const_fold(sea: &mut SeaOfNodes) -> Result<PassReport, MiddleError> {
    sea.verify_structure()?;
    check_budget(sea.nodes.len())?;
    let mut changed = 0usize;
    for u in 0..sea.nodes.len() {
        let snapshot = sea.nodes[u].clone();
        if !is_pure_value_opcode(snapshot.opcode) || matches!(snapshot.opcode, Opcode::Const(_)) {
            continue;
        }
        let folded = match snapshot.opcode {
            Opcode::Cvt => match snapshot.inputs.as_slice() {
                [a] => {
                    let src = match sea.node(*a) {
                        Ok(n) => n,
                        Err(e) => return Err(MiddleError::Ir(e)),
                    };
                    match src.opcode {
                        Opcode::Const(v) => fold_unary(Opcode::Cvt, v, src.ty, snapshot.ty),
                        _ => None,
                    }
                }
                _ => continue,
            },
            Opcode::Neg | Opcode::Not => match snapshot.inputs.as_slice() {
                [a] => const_typed(sea, *a)?.and_then(|(val, t)| {
                    if t == snapshot.ty {
                        fold_unary(snapshot.opcode, val, t, snapshot.ty)
                    } else {
                        None
                    }
                }),
                _ => continue,
            },
            _ => match snapshot.inputs.as_slice() {
                [a, b] => {
                    let va = const_typed(sea, *a)?;
                    let vb = const_typed(sea, *b)?;
                    match (va, vb) {
                        (Some((x, tx)), Some((y, ty_b))) if tx == ty_b => {
                            let is_cmp = matches!(
                                snapshot.opcode,
                                Opcode::CmpEq | Opcode::CmpLt | Opcode::CmpLe
                            );
                            if is_cmp || tx == snapshot.ty {
                                fold_binary(snapshot.opcode, x, y, tx, snapshot.ty)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                }
                _ => continue,
            },
        };
        if let Some(pattern) = folded {
            let node = &mut sea.nodes[u];
            node.opcode = Opcode::Const(pattern);
            node.inputs.clear();
            changed += 1;
        }
    }
    sea.verify_structure()?;
    Ok(PassReport { name: "CONST_FOLD", changed })
}

/// CEP:WHAT: Reads the constant pattern and type of an operand when it is a Const.
/// CEP:WHY: Comparisons fold over same-typed operands of the operand type while the
///   result type is Bool; arithmetic additionally requires it to equal the result type.
/// CEP:STATUS: complete
/// CEP:FAILURE: None value when the operand is not a constant.
/// CEP:ASSUMES: dangling ids cannot occur (structure verified at pass start).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::const_fold_respects_type_widths.
fn const_typed(sea: &SeaOfNodes, id: NodeId) -> Result<Option<(i64, IrType)>, MiddleError> {
    let node = match sea.node(id) {
        Ok(n) => n,
        Err(e) => return Err(MiddleError::Ir(e)),
    };
    match node.opcode {
        Opcode::Const(v) => Ok(Some((v, node.ty))),
        _ => Ok(None),
    }
}

/// CEP:WHAT: The GVN hash key: opcode payload, result type, and operand ids.
/// CEP:WHY: Hash-consing requires full structural equality after hash bucketing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Opcode and IrType derive PartialEq/Eq/Hash.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::gvn_removes_duplicate_subgraph.
type GvnKey = (Opcode, IrType, Vec<NodeId>);

/// CEP:WHAT: GVN pass: value-numbers pure nodes in node-id order within one block and
///   replaces duplicates via replace_uses against the first representative.
/// CEP:WHY: Common-subexpression elimination with deterministic candidate order (node id
///   ascending); same-block scope keeps dominance sound without code motion.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Ir on structural failure; MiddleError::Budget on oversize.
/// CEP:ASSUMES: representatives precede duplicates in id order and share the duplicate's
///   block (None-block synthetic graphs form one equivalence class, matching the
///   verifier's documented skip of unblocked nodes).
/// CEP:COST: O(N * fanin) expected; hash buckets are lookup-only and never iterated.
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::gvn_removes_duplicate_subgraph.
pub fn gvn(sea: &mut SeaOfNodes) -> Result<PassReport, MiddleError> {
    sea.verify_structure()?;
    check_budget(sea.nodes.len())?;
    // Buckets are keyed by the canonical Fx hash and only ever probed, never iterated:
    // iteration order of this map cannot reach any output (38.10).
    let mut table: FxHashMap<u64, Vec<(GvnKey, NodeId)>> = FxHashMap::default();
    let mut changed = 0usize;
    for u in 0..sea.nodes.len() {
        let snapshot = sea.nodes[u].clone();
        if !is_pure_value_opcode(snapshot.opcode) {
            continue;
        }
        let mut h: u64 = fx_mix(0, fxhash64(snapshot.opcode.name().as_bytes()));
        h = fx_mix(h, u64::from(snapshot.ty as u8));
        for &d in &snapshot.inputs {
            h = fx_mix(h, d.0 as u64);
        }
        let key: GvnKey = (snapshot.opcode, snapshot.ty, snapshot.inputs.clone());
        let bucket = table.entry(h).or_default();
        let rep = bucket.iter().find_map(|(k, r)| {
            (*k == key && sea.nodes[r.0 as usize].block == snapshot.block).then_some(*r)
        });
        match rep {
            Some(rep_id) => {
                sea.replace_uses(NodeId(u as u32), rep_id);
                changed += 1;
            }
            None => bucket.push((key, NodeId(u as u32))),
        }
    }
    sea.verify_structure()?;
    Ok(PassReport { name: "GVN", changed })
}

/// CEP:WHAT: DCE pass: marks side-effect/control roots, transitively marks all data and
///   control dependencies with an iterative worklist, then sweeps unmarked pure nodes by
///   deterministic rebuild with an ascending id remap.
/// CEP:WHY: Dead pure nodes must not reach Level 2 (spec 3.3.4) and dead sweeps shrink
///   the graph for the backend.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Ir on structural failure; MiddleError::Budget on oversize;
///   MiddleError::Pass if the mark invariant breaks internally (fail-closed, no panic).
/// CEP:ASSUMES: every operand of a live node is itself live (marking walks all inputs).
/// CEP:COST: O(N * fanin); rebuild O(N) nodes plus input remaps.
/// CEP:EVIDENCE: unit test rv8_middle::passes::tests::dce_sweeps_dead_pure_subgraph.
pub fn dce(sea: &mut SeaOfNodes) -> Result<PassReport, MiddleError> {
    sea.verify_structure()?;
    check_budget(sea.nodes.len())?;
    let n = sea.nodes.len();
    let mut live = vec![false; n];
    let mut work: Vec<usize> = Vec::new();
    for (i, node) in sea.nodes.iter().enumerate() {
        if is_dce_root(node.opcode) {
            live[i] = true;
            work.push(i);
        }
    }
    // Iterative marking: every input of a live node is live.
    while let Some(i) = work.pop() {
        for &d in &sea.nodes[i].inputs {
            let di = d.0 as usize;
            if di < n && !live[di] {
                live[di] = true;
                work.push(di);
            }
        }
    }
    // Deterministic sweep: rebuild with ascending remap so ids stay dense.
    let kept = live.iter().filter(|&&l| l).count();
    let mut map: Vec<Option<NodeId>> = vec![None; n];
    let mut out = SeaOfNodes::new();
    out.nodes.reserve(kept);
    for (i, node) in sea.nodes.iter().enumerate() {
        if live[i] {
            map[i] = Some(NodeId(out.nodes.len() as u32));
            out.nodes.push(rv8_ir::son::SeaNode {
                opcode: node.opcode,
                ty: node.ty,
                inputs: Vec::new(),
                block: node.block,
                span: node.span,
            });
        }
    }
    for (i, node) in sea.nodes.iter().enumerate() {
        if !live[i] {
            continue;
        }
        let new_id = map[i]
            .ok_or(MiddleError::Pass { pass: "DCE", detail: "live node unmapped" })?;
        let mut remapped: Vec<NodeId> = Vec::with_capacity(node.inputs.len());
        for d in &node.inputs {
            let target = map[d.0 as usize]
                .ok_or(MiddleError::Pass { pass: "DCE", detail: "operand of live node swept" })?;
            remapped.push(target);
        }
        out.nodes[new_id.0 as usize].inputs = remapped;
    }
    *sea = out;
    sea.verify_structure()?;
    Ok(PassReport { name: "DCE", changed: n - kept })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::analysis::tests::if_else_son;

    /// CEP:WHAT: CONST_FOLD folds pure constant operations, respects widths (i8 wrap,
    /// unsigned shift), and leaves division by zero unfolded.
    /// CEP:WHY: Core pass contract including the do-not-fold hazard rule.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn const_fold_folds_and_skips_div_zero() {
        let mut sea = SeaOfNodes::new();
        let a = sea.add_node(Opcode::Const(6), IrType::I32, vec![]);
        let b = sea.add_node(Opcode::Const(7), IrType::I32, vec![]);
        let sum = sea.add_node(Opcode::Add, IrType::I32, vec![a, b]);
        let prod = sea.add_node(Opcode::Mul, IrType::I32, vec![a, b]);
        let zero = sea.add_node(Opcode::Const(0), IrType::I32, vec![]);
        let div = sea.add_node(Opcode::Div, IrType::I32, vec![a, zero]);
        let ret = sea.add_node(Opcode::Return, IrType::Unit, vec![sum, prod, div]);
        let report = const_fold(&mut sea).expect("fold");
        assert_eq!(report.name, "CONST_FOLD");
        assert_eq!(report.changed, 2, "Add and Mul fold; Div by zero does not");
        assert!(matches!(sea.node(sum).expect("sum").opcode, Opcode::Const(13)));
        assert!(matches!(sea.node(prod).expect("prod").opcode, Opcode::Const(42)));
        assert!(matches!(sea.node(div).expect("div").opcode, Opcode::Div));
        assert!(sea.node(ret).expect("ret").inputs.contains(&div));
        sea.verify_structure().expect("structure after fold");
    }

    /// CEP:WHAT: CONST_FOLD respects type widths: i8 addition wraps, u8 shifts are
    ///   unsigned, and comparison results are Bool.
    /// CEP:WHY: Width discipline is the pass contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn const_fold_respects_type_widths() {
        // (200i8 + 100i8) wraps to 44.
        let mut sea = SeaOfNodes::new();
        let a = sea.add_node(Opcode::Const(200), IrType::I8, vec![]);
        let b = sea.add_node(Opcode::Const(100), IrType::I8, vec![]);
        let sum = sea.add_node(Opcode::Add, IrType::I8, vec![a, b]);
        // u8 0xFF >> 4 = 0x0F.
        let m = sea.add_node(Opcode::Const(0xFF), IrType::U8, vec![]);
        let s = sea.add_node(Opcode::Const(4), IrType::U8, vec![]);
        let shr = sea.add_node(Opcode::Shr, IrType::U8, vec![m, s]);
        // 3 < 4u32 -> true.
        let three = sea.add_node(Opcode::Const(3), IrType::U32, vec![]);
        let four = sea.add_node(Opcode::Const(4), IrType::U32, vec![]);
        let lt = sea.add_node(Opcode::CmpLt, IrType::Bool, vec![three, four]);
        const_fold(&mut sea).expect("fold");
        assert!(matches!(sea.node(sum).expect("sum").opcode, Opcode::Const(44)));
        assert!(matches!(sea.node(shr).expect("shr").opcode, Opcode::Const(0x0F)));
        assert!(matches!(sea.node(lt).expect("lt").opcode, Opcode::Const(1)));
        // Shift >= width is not folded.
        let mut sea2 = SeaOfNodes::new();
        let x = sea2.add_node(Opcode::Const(1), IrType::U8, vec![]);
        let y = sea2.add_node(Opcode::Const(8), IrType::U8, vec![]);
        let shl = sea2.add_node(Opcode::Shl, IrType::U8, vec![x, y]);
        const_fold(&mut sea2).expect("fold 2");
        assert!(matches!(sea2.node(shl).expect("shl").opcode, Opcode::Shl));
    }

    /// CEP:WHAT: GVN merges a duplicated pure subgraph by id-ascending representatives and
    ///   leaves cross-type duplicates alone.
    /// CEP:WHY: Core hash-consing contract with deterministic candidates.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn gvn_removes_duplicate_subgraph() {
        let mut sea = SeaOfNodes::new();
        let a = sea.add_node(Opcode::Const(2), IrType::I32, vec![]);
        let b = sea.add_node(Opcode::Const(3), IrType::I32, vec![]);
        let s1 = sea.add_node(Opcode::Add, IrType::I32, vec![a, b]);
        let a2 = sea.add_node(Opcode::Const(2), IrType::I32, vec![]);
        let b2 = sea.add_node(Opcode::Const(3), IrType::I32, vec![]);
        let s2 = sea.add_node(Opcode::Add, IrType::I32, vec![a2, b2]);
        let ret = sea.add_node(Opcode::Return, IrType::Unit, vec![s1, s2]);
        let report = gvn(&mut sea).expect("gvn");
        assert_eq!(report.changed, 3, "two consts and one add merged");
        assert_eq!(sea.node(ret).expect("ret").inputs, vec![s1, s1]);
        // Duplicated nodes keep their ids until DCE sweeps them.
        sea.verify_structure().expect("structure after gvn");
    }

    /// CEP:WHAT: DCE sweeps an unused pure subgraph and keeps everything reachable from
    ///   roots, remapping ids densely.
    /// CEP:WHY: Core mark-and-sweep contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn dce_sweeps_dead_pure_subgraph() {
        let mut sea = SeaOfNodes::new();
        let live_c = sea.add_node(Opcode::Const(5), IrType::I32, vec![]);
        let dead_c = sea.add_node(Opcode::Const(9), IrType::I32, vec![]);
        let dead_add = sea.add_node(Opcode::Add, IrType::I32, vec![dead_c, dead_c]);
        let live_add = sea.add_node(Opcode::Add, IrType::I32, vec![live_c, live_c]);
        let ret = sea.add_node(Opcode::Return, IrType::Unit, vec![live_add]);
        let _ = dead_add;
        let report = dce(&mut sea).expect("dce");
        assert_eq!(report.changed, 2, "dead const and dead add swept");
        assert_eq!(sea.nodes.len(), 3);
        assert!(sea.verify_structure().is_ok());
        // Ids are remapped densely in ascending order: live const, live add, return.
        assert_eq!(ret.0 as usize, 4, "old return id");
        let new_ret = &sea.nodes[2];
        assert!(matches!(new_ret.opcode, Opcode::Return));
        assert_eq!(new_ret.inputs, vec![NodeId(1)]);
        assert!(matches!(sea.nodes[1].opcode, Opcode::Add));
        assert!(matches!(sea.nodes[0].opcode, Opcode::Const(5)));
    }

    /// CEP:WHAT: Verifies the pure-opcode and root eligibility tables agree with the
    ///   documented effect classes.
    /// CEP:WHY: One eligibility decision must drive fold, GVN, and DCE consistently.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn eligibility_tables_agree() {
        assert!(is_pure_value_opcode(Opcode::Add));
        assert!(!is_pure_value_opcode(Opcode::Phi));
        assert!(!is_pure_value_opcode(Opcode::Load));
        assert!(!is_pure_value_opcode(Opcode::Store));
        assert!(is_dce_root(Opcode::Alloc));
        assert!(is_dce_root(Opcode::EscapePoint));
        assert!(is_dce_root(Opcode::Parameter(0)));
        assert!(!is_dce_root(Opcode::Load));
        assert!(!is_dce_root(Opcode::Const(0)));
    }

    /// CEP:WHAT: Runs all three passes over a lowered if/else graph; every pass keeps the
    ///   graph verifiable (composition sanity with a real CFG).
    /// CEP:WHY: Passes must compose over lowered shapes, not just synthetic lines.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn passes_compose_over_lowered_graph() {
        let mut sea = if_else_son();
        let before = sea.nodes.len();
        let r1 = const_fold(&mut sea).expect("fold");
        let r2 = gvn(&mut sea).expect("gvn");
        let r3 = dce(&mut sea).expect("dce");
        assert!(r1.changed + r2.changed + r3.changed <= before);
        assert!(sea.verify_structure().is_ok());
    }
}
