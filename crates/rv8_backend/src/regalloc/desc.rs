// CEP:FILE: crates/rv8_backend/src/regalloc/desc.rs
// CEP:WHAT: The bridge between ISL target descriptions and the allocator core:
//   `Plan` (per-class lane tables with savedness and divide-clobber roles, the cost
//   table, and the ABI registers) plus the adapters `x86_64_plan` (the shipped
//   description) and `plan_of` (any description). This is where target facts become
//   allocator inputs, and the ONLY place the two meet.
// CEP:WHY: One reggolloc serves every target: the core consumes a Plan; a target is
//   an ISL file. Cost scaling rounds the ISL's decimal costs onto the integer cost
//   grid the flow solver requires (COST_SCALE points per unit).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: BackendError::UnsupportedPattern when a description has fewer than two
//   classes (the allocator solves GPR and FPR); parse errors surface from rv8_isl.
// CEP:ASSUMES: the first described class is the integer class (ABI roles), the second
//   the floating-point class; at most 64 lanes per class.
// CEP:COST: one ISL parse per plan; the plan is cheap to cache.
// CEP:EVIDENCE: the cross-check tests pin every x86-64 lane and cost against the
//   hard-coded abi tables; the toy-target test proves two descriptions share the one
//   core.
use crate::error::BackendError;
use rv8_ir::machir::PhysReg;
use rv8_isl::TargetDescription;

/// CEP:WHAT: The integer cost grid the flow solver consumes; one cost unit of the
///   published tables equals COST_SCALE network points.
/// CEP:WHY: Min-cost flow needs integers; the ISL's decimal costs round onto this
///   grid, and the brute-force oracles price assignments on the identical table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: costs are small positive decimals; rounding is deterministic.
/// CEP:COST: POD.
/// CEP:EVIDENCE: the corpora price with the same constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CostTable {
    /// Cost of one def-connection or use-connection edge.
    pub move_cost: i64,
    /// Cost of routing a commodity through the spill edge.
    pub spill_cost: i64,
    /// Extra cost on the def connection into a callee-saved register.
    pub save_restore_cost: i64,
    /// Cost of the emergency spill edge available to pinned ranges.
    pub pinned_spill_cost: i64,
}

/// CEP:WHAT: One register class as the allocator sees it: lanes in RegId order plus
///   the per-lane roles the barriers and costs consult.
/// CEP:WHY: Target facts as data for the one shared core.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at most 64 lanes.
/// CEP:COST: small vectors.
/// CEP:EVIDENCE: cross-check tests.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassPlan {
    /// Class name from the description.
    pub name: String,
    /// Lanes in RegId order; `RegId(i)` maps to `lanes[i]`.
    pub lanes: Vec<PhysReg>,
    /// Per-lane callee-saved flag (save/restore cost, prologue pushes).
    pub callee_saved: Vec<bool>,
    /// Per-lane divide-clobber flag (the cqo/idiv window blocks these lanes).
    pub div_clobber: Vec<bool>,
}

/// CEP:WHAT: The whole allocator-facing target: costs, the integer and floating-point
///   class plans, and the ABI registers.
/// CEP:WHY: THE input of allocate_with; every target shares this one shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction only through plan_of/x86_64_plan.
/// CEP:ASSUMES: none
/// CEP:COST: small vectors.
/// CEP:EVIDENCE: cross-check tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The cost table (integer grid).
    pub costs: CostTable,
    /// [integer class, floating-point class].
    pub classes: [ClassPlan; 2],
    /// The ABI return register.
    pub abi_return: PhysReg,
    /// The ABI argument registers in ABI order.
    pub abi_args: Vec<PhysReg>,
}

/// CEP:WHAT: Scales one ISL decimal cost onto the integer grid (round-half-away,
///   deterministic for the small decimals targets use).
/// CEP:WHY: The solver's integer arithmetic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: cross-check tests pin the rounding for the shipped costs.
fn scale(cost: f64) -> i64 {
    (cost * super::COST_SCALE as f64).round() as i64
}

/// CEP:WHAT: Adapts any parsed target description to an allocator Plan.
/// CEP:WHY: The single meeting point of ISL data and allocator inputs.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnsupportedPattern when fewer than two classes are described.
/// CEP:ASSUMES: the first class hosts the ABI roles (validated by rv8_isl).
/// CEP:COST: O(lanes).
/// CEP:EVIDENCE: the toy-target test and the x86-64 cross-checks.
pub fn plan_of(desc: &TargetDescription) -> Result<Plan, BackendError> {
    if desc.classes.len() < 2 {
        return Err(BackendError::UnsupportedPattern {
            detail: format!(
                "target {} describes {} classes; the allocator needs an integer and a \
                 floating-point class",
                desc.name,
                desc.classes.len()
            ),
        });
    }
    let mut classes = Vec::with_capacity(2);
    for c in desc.classes.iter().take(2) {
        classes.push(ClassPlan {
            name: c.name.clone(),
            lanes: c.lanes.iter().map(|l| PhysReg(l.enc)).collect(),
            callee_saved: c.lanes.iter().map(|l| l.callee_saved).collect(),
            div_clobber: c.lanes.iter().map(|l| l.div_clobber).collect(),
        });
    }
    let ret_lane = desc.lane(&desc.classes[0].name, &desc.return_reg)
        .expect("rv8_isl validated the return register");
    let args = desc
        .args
        .iter()
        .map(|a| {
            let l = desc
                .lane(&desc.classes[0].name, a)
                .expect("rv8_isl validated the argument registers");
            PhysReg(l.enc)
        })
        .collect();
    Ok(Plan {
        costs: CostTable {
            move_cost: scale(desc.classes[0].move_cost),
            spill_cost: scale(desc.classes[0].spill_cost),
            save_restore_cost: super::SAVE_RESTORE_COST,
            pinned_spill_cost: super::PINNED_SPILL_COST,
        },
        classes: [classes.remove(0), classes.remove(0)],
        abi_return: PhysReg(ret_lane.enc),
        abi_args: args,
    })
}

/// CEP:WHAT: The shipped x86-64 plan, derived from the embedded ISL description.
/// CEP:WHY: The production target; allocate() delegates here.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect on the shipped file only if it stops parsing, which
///   the rv8_isl and cross-check tests make a build failure in practice.
/// CEP:ASSUMES: the shipped description is valid.
/// CEP:COST: one parse per call (cold; cache the Plan in hot drivers).
/// CEP:EVIDENCE: x86_64_plan_matches_abi_tables.
pub fn x86_64_plan() -> Plan {
    let desc = rv8_isl::x86_64().expect("shipped ISL parses");
    plan_of(&desc).expect("shipped ISL adapts")
}

/// CEP:WHAT: A four-lane toy target for the shared-core demonstration: the same LIR
///   allocated under this plan spills where x86-64 does not.
/// CEP:WHY: Proves the ISL drives the one core (user law 3 and 4).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect on the shipped toy file only.
/// CEP:ASSUMES: the toy file is valid.
/// CEP:COST: one parse per call (cold).
/// CEP:EVIDENCE: allocate_with_two_targets_share_the_core.
pub fn toy4_plan() -> Plan {
    let desc = rv8_isl::parse(rv8_isl::TOY4_ISL).expect("toy ISL parses");
    plan_of(&desc).expect("toy ISL adapts")
}

/// CEP:WHAT: Cross-checks the x86-64 plan against the hard-coded abi tables.
/// CEP:WHY: The ISL is the description of record; this test makes drift between the
///   description and the encoder tables a build failure.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[cfg(test)]
pub mod checks {
    use super::*;
    use crate::abi;

    /// CEP:WHAT: Every lane, role, and cost of the ISL plan matches the hard-coded
    ///   tables.
    /// CEP:WHY: Description-of-record enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn x86_64_plan_matches_abi_tables() {
        let plan = x86_64_plan();
        assert_eq!(plan.classes[0].lanes, abi::ALLOCATABLE_GPRS.to_vec());
        assert_eq!(plan.classes[1].lanes, abi::ALLOCATABLE_FPRS.to_vec());
        for (i, &p) in abi::ALLOCATABLE_GPRS.iter().enumerate() {
            assert_eq!(plan.classes[0].callee_saved[i], abi::is_callee_saved(p));
        }
        assert_eq!(plan.abi_return, abi::RETURN_REGISTER);
        assert_eq!(plan.abi_args, abi::ARG_REGISTERS.to_vec());
        assert_eq!(plan.costs.move_cost, crate::regalloc::MOVE_COST);
        assert_eq!(plan.costs.spill_cost, crate::regalloc::SPILL_COST);
        // The divide clobbers are exactly RAX and RDX.
        let div: Vec<PhysReg> = plan.classes[0]
            .lanes
            .iter()
            .zip(plan.classes[0].div_clobber.iter())
            .filter(|(_, &d)| d)
            .map(|(&p, _)| p)
            .collect();
        assert_eq!(div, vec![abi::RAX, abi::RDX]);
    }
}
