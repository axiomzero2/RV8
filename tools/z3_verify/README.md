# CEP:FILE: tools/z3_verify/README.md
# CEP:WHAT: Z3 verification of the RV8-137 soundness arguments: the loop-extended
#   lane model (cyclic soundness), the phi-copy sequencer (parallel semantics on all
#   copy sets up to four variables), and hop-free timeline interference freedom.
#   Complements verification/z3_wrap_soundness.py (the wrap-liveness proof).
# CEP:WHY: The structural arguments behind the fixes deserve machine-checked
#   evidence; each model is proved by refutation (assert the rule and the negation of
#   the guarantee; unsat proves the implication).
# CEP:CLASS: CEP-2
# CEP:STATUS: complete
# CEP:OWNER: axiomzero2
# CEP:TICKET: RV8-137
# CEP:FAILURE: a REFUTED verdict exits nonzero.
# CEP:ASSUMES: python3 with the z3-solver package (pip install z3-solver); a
#   verification tool, not a build dependency (the Cargo workspace stays dependency-free).
# CEP:COST: ~30 seconds.
# CEP:EVIDENCE: z3_rv8_137_report.txt records the run; rerun with
#   `python3 tools/z3_verify/z3_rv8_137.py`.
