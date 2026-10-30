# CEP:FILE: targets/toy4.isl
# CEP:WHAT: A four-lane demonstration target: proves the ISL drives the one shared
#   allocator core (the same LIR spills here and not on x86-64).
# CEP:WHY: User law 3 and 4: each target shares exactly one reggolloc; the ISL
#   carries target descriptions.
# CEP:CLASS: CEP-1
# CEP:STATUS: complete
# CEP:FAILURE: parse or validation errors name the offending line.
# CEP:ASSUMES: encodings occupy the machir GPR numbering for interoperability.
# CEP:COST: cold.
# CEP:EVIDENCE: allocate_with_two_targets_share_the_core.
target "toy4" {
    class gpr {
        move_cost 1.0
        spill_cost 100.0
        lane a0 enc 0 caller-saved return arg 0 div-clobber
        lane a1 enc 1 caller-saved arg 1 div-clobber
        lane s0 enc 2 callee-saved
        lane s1 enc 3 callee-saved
    }
    class fpr {
        move_cost 1.0
        spill_cost 100.0
        lane v0 enc 16 caller-saved
        lane v1 enc 17 caller-saved
    }
    abi return a0 args a0 a1
    frame alignment 16
}
