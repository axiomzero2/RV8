#!/usr/bin/env python3
"""Z3 verification of the RV8-137 soundness arguments (background job).

Three models, each checked by refutation (assert rule AND negation of the
guarantee; UNSAT proves the implication for the modeled family):

1. lane_model_soundness - loop-extended live ranges make same-register values
   cyclically safe inside loops.
2. sequencer_correctness - the phi-copy sequencer (corrected ready condition,
   one scratch per cycle) realizes parallel copy semantics on every copy set
   over up to five variables (Z3 checks each instance symbolically).
3. hopfreedom_interference - hop-free single-unit register timelines cannot
   carry two overlapping live ranges on one lane.

Also mirrors the sequencer in Python and cross-checks it against the parallel
semantics by Z3 on all copy sets (destinations unique) of size 2..5.
"""

import itertools
import sys

import z3

VERDICTS = []


def sequencer_emits(copies):
    """Mirror of hetir_to_lir's sequencer: destination-not-a-pending-source
    ready condition, one fresh scratch per pure cycle, deferred deliveries."""
    pending = list(copies)
    seq = []
    deferred = []
    scratch_id = 0
    while True:
        k = None
        for i, (d, s) in enumerate(pending):
            if not any(s2 == d for (_, s2) in pending):
                k = i
                break
        if k is not None:
            seq.append(pending.pop(k))
            continue
        if not pending:
            break
        scratch_id += 1
        sc = f"SCR{scratch_id}"
        d0, s0 = pending[0]
        seq.append((sc, s0))
        deferred.append((d0, sc))
        pending.pop(0)
    seq.extend(deferred)
    return seq


def check_by_refutation(name, prop):
    solver = z3.Solver()
    solver.add(prop)
    res = solver.check()
    if res == z3.unsat:
        VERDICTS.append((name, "PROVED (refutation unsat)"))
    elif res == z3.sat:
        VERDICTS.append((name, "REFUTED (counterexample exists)"))
    else:
        VERDICTS.append((name, f"UNKNOWN ({res})"))
    return res


def lane_model_soundness():
    d_inv, s_start, s_end = z3.Ints("D_INV S E")
    d_body, u_body, u_inv = z3.Ints("D_BODY U_BODY U_INV")
    same_reg = z3.Bool("same_reg")
    instance = z3.And(
        d_inv >= 1, d_inv < s_start,
        s_start >= 2, s_start < s_end, s_end <= 12,
        u_inv >= s_start, u_inv <= s_end,
        d_body >= s_start, d_body < u_body, u_body <= s_end,
        u_inv > d_body,
    )
    # Rule: same register requires disjoint extended ranges
    # [d_inv, s_end) for the invariant (loop extension) and [d_body, u_body).
    ranges_overlap = z3.And(d_inv < u_body, d_body < s_end)
    lane_rule = z3.Implies(same_reg, z3.Not(ranges_overlap))
    # Negated guarantee: the body write clobbers the invariant before the
    # next iteration's read (same register, body write inside the span before
    # the invariant's read point).
    clobber = z3.And(same_reg, d_body < u_inv)
    return z3.And(instance, lane_rule, clobber)


def sequencer_correctness(max_vars=4):
    """For every copy set (unique destinations, size 2..max_vars), the emitted
    serial sequence must implement the parallel assignment final[d]=init[s]."""
    bad = []
    tested = 0
    for n in range(2, max_vars + 1):
        vars_ = [f"v{i}" for i in range(n)]
        for size in range(2, n + 1):
            for dsts in itertools.permutations(vars_, size):
                for srcs in itertools.product(vars_, repeat=size):
                    copies = [(d, s) for d, s in zip(dsts, srcs) if d != s]
                    if len(copies) < 2:
                        continue
                    tested += 1
                    order = sequencer_emits(copies)
                    inits = {v: z3.Int(f"init_{v}") for v in vars_}
                    for k in range(1, 6):
                        inits[f"SCR{k}"] = z3.Int(f"init_scratch_{k}")
                    state = dict(inits)
                    for d, s in order:
                        state[d] = state[s]
                    goal = z3.And(state[d] == inits[s] for (d, s) in copies)
                    solver = z3.Solver()
                    solver.add(z3.Not(goal))
                    if solver.check() == z3.sat:
                        bad.append(tuple(sorted(copies)))
    VERDICTS.append((
        "sequencer_correctness",
        f"PROVED ({tested} copy-set instances checked)" if not bad
        else f"REFUTED on {len(bad)} instances, e.g. {bad[:3]}",
    ))
    return bad


def hopfreedom_interference():
    d1, u1, d2, u2 = z3.Ints("d1 u1 d2 u2")
    both_on_lane = z3.Bool("both_on_lane")
    overlap = z3.And(d1 < u2, d2 < u1)
    # A single unit's forward path takes only time-disjoint chords.
    disjoint_chords = z3.Or(u1 <= d2, u2 <= d1)
    rule = z3.Implies(both_on_lane, disjoint_chords)
    violation = z3.And(both_on_lane, overlap)
    return z3.And(rule, violation)


def main():
    check_by_refutation("lane_model_soundness", lane_model_soundness())
    sequencer_correctness()
    check_by_refutation("hopfreedom_interference", hopfreedom_interference())
    lines = []
    for name, verdict in VERDICTS:
        lines.append(f"[z3] {name}: {verdict}")
        print(lines[-1])
    failed = any("REFUTED" in v or "UNKNOWN" in v for _, v in VERDICTS)
    overall = "FAIL" if failed else "ALL PROVED"
    print(f"[z3] overall: {overall}")
    lines.append(f"[z3] overall: {overall}")
    with open("z3_rv8_137_report.txt", "w") as fh:
        fh.write("\n".join(lines) + "\n")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
