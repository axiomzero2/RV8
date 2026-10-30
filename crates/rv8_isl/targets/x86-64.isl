# CEP:FILE: targets/x86-64.isl
# CEP:WHAT: The x86-64 target description in ISL: register lanes per class in RegId
#   order with hardware encodings and roles, per-class costs, the ABI register
#   assignment, and the frame shape. The allocator and the cross-check tests consume
#   this file; the hard-coded tables in rv8_backend::abi must match it exactly.
# CEP:WHY: Target facts as data: a target is a description, not a code path.
# CEP:CLASS: CEP-1
# CEP:STATUS: complete
# CEP:FAILURE: parse or validation errors name the offending line.
# CEP:ASSUMES: SysV AMD64 register conventions; enc numbers match the machir physical
#   register numbering used by rv8_backend::abi.
# CEP:COST: cold; parsed once per allocation request that does not cache the result.
# CEP:EVIDENCE: rv8_backend::desc cross-check tests pin every lane against abi.rs.
target "x86-64" {
    class gpr {
        move_cost 1.0
        spill_cost 100.0
        lane rax enc 0 caller-saved return div-clobber
        lane rcx enc 1 caller-saved arg 3
        lane rdx enc 2 caller-saved arg 2 div-clobber
        lane rbx enc 3 callee-saved
        lane rbp enc 5 callee-saved
        lane rsi enc 6 caller-saved arg 1
        lane rdi enc 7 caller-saved arg 0
        lane r8 enc 8 caller-saved arg 4
        lane r9 enc 9 caller-saved arg 5
        lane r10 enc 10 caller-saved
        lane r11 enc 11 caller-saved
        lane r12 enc 12 callee-saved
        lane r13 enc 13 callee-saved
        lane r14 enc 14 callee-saved
        lane r15 enc 15 callee-saved
    }
    class fpr {
        move_cost 1.0
        spill_cost 100.0
        lane xmm0 enc 16 caller-saved
        lane xmm1 enc 17 caller-saved
        lane xmm2 enc 18 caller-saved
        lane xmm3 enc 19 caller-saved
        lane xmm4 enc 20 caller-saved
        lane xmm5 enc 21 caller-saved
        lane xmm6 enc 22 caller-saved
        lane xmm7 enc 23 caller-saved
        lane xmm8 enc 24 caller-saved
        lane xmm9 enc 25 caller-saved
        lane xmm10 enc 26 caller-saved
        lane xmm11 enc 27 caller-saved
        lane xmm12 enc 28 caller-saved
        lane xmm13 enc 29 caller-saved
        lane xmm14 enc 30 caller-saved
        lane xmm15 enc 31 caller-saved
    }
    abi return rax args rdi rsi rdx rcx r8 r9
    frame alignment 16 red-zone-leaf
}
