#!/usr/bin/env python3
# CEP:FILE: tools/fv_zero_sorry.py
# CEP:WHAT: The FV-0 zero-sorry source gate.  Scans Lean sources and fails on
#   any proof-hole token (`sorry`, `sorryAx`, `mkSorry`) that appears in actual
#   code.  Comments (`--` line comments and nested `/- ... -/` block comments)
#   and string literals are stripped first, so documentation that *describes*
#   the gate (e.g. RV8FV/Audit.lean's own header) cannot false-positive, while
#   a real hole anywhere in code is still caught — including indirect
#   sorry-minting through `mkSorry` inside metaprograms.
# CEP:WHY: CEP&CC 54.2 requires zero-sorry FV-0 results.  The machine audit in
#   RV8FV/Audit.lean (a total collectAxioms sweep over the RV8FV namespace) is
#   the hard gate: any imported `sorry` produces `sorryAx` and fails `lake
#   build`.  This scanner is the complementary source-level gate: it covers
#   every file handed to it whether or not it is imported, and unlike a naive
#   grep it reads Lean syntax, so prose mentioning the gate never trips it and
#   genuine holes are never skipped.
# CEP:CLASS: FV-0
# CEP:STATUS: complete
# CEP:OWNER: axiomzero2
# CEP:FAILURE: exits 1 if any proof-hole token survives comment/string
#   stripping; exits 1 if --self-test fails.  Exits 0 only when every scanned
#   file is code-clean.
# CEP:ASSUMES: python3 (stdlib only); paths are Lean files or directories
#   (directories are walked recursively for *.lean).  The CI Lean gate passes
#   RV8FV RV8FV.lean SmokeMain.lean from verification/lean, mirroring the
#   lakefile targets.  Char literals containing comment syntax (e.g. '/-') are
#   not modeled and would be treated as comment starts; none occur in RV8FV.
# CEP:COST: milliseconds; no network, no build.
# CEP:EVIDENCE: run `python3 tools/fv_zero_sorry.py --self-test` (11 cases) and
#   the CI Lean gate step; the Audit.lean false-positive that motivated the
#   comment-aware scanner is regression case S10 below.

import os
import re
import sys

PROG = "fv-zero-sorry"
VIOLATION = "FV-0 violation: sorry found (CEP&CC 54.3.1)"

# Proof-hole tokens, longest first so `sorryAx` wins over `sorry`.
HOLE_RE = re.compile(r"\b(?:sorryAx|mkSorry|sorry)\b")


def strip_lean(src):
    """Remove Lean comments and string-literal contents, keep every newline.

    Line numbers of surviving code therefore match the original file, so
    reported locations point at the real line.  Handles nested block
    comments, line comments, and double-quoted strings with backslash
    escapes (string contents are dropped so prose inside messages never
    matches; the quotes themselves are kept).
    """
    out = []
    i, n = 0, len(src)
    depth = 0
    while i < n:
        c = src[i]
        if depth > 0:
            if src.startswith("/-", i):
                depth += 1
                i += 2
            elif src.startswith("-/", i):
                depth -= 1
                i += 2
            else:
                if c == "\n":
                    out.append(c)
                i += 1
            continue
        if src.startswith("/-", i):
            depth = 1
            i += 2
            continue
        if src.startswith("--", i):
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == '"':
            out.append(c)
            i += 1
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == '"':
                    i += 1
                    break
                if src[i] == "\n":
                    out.append("\n")
                i += 1
            out.append('"')
            continue
        out.append(c)
        i += 1
    return "".join(out)


def scan_text(relpath, text):
    """Yield (lineno, line) for code lines still carrying a hole token."""
    code = strip_lean(text)
    for lineno, line in enumerate(code.splitlines(), 1):
        if HOLE_RE.search(line):
            yield lineno, line.strip()


def collect_paths(args):
    """Expand file/directory arguments into a sorted list of .lean files."""
    files = []
    for arg in args:
        if os.path.isdir(arg):
            for root, _dirs, names in os.walk(arg):
                for name in sorted(names):
                    if name.endswith(".lean"):
                        files.append(os.path.join(root, name))
        else:
            files.append(arg)
    return sorted(files)


def run_scan(paths):
    hits = 0
    scanned = 0
    for path in paths:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
        scanned += 1
        for lineno, line in scan_text(path, text):
            print(f"{path}:{lineno}:{line}")
            hits += 1
    if hits:
        print(f"{PROG}: {VIOLATION}")
        print(f"{PROG}: {hits} proof-hole token(s) in code across {scanned} file(s)")
        return 1
    print(f"{PROG}: clean — {scanned} Lean file(s) scanned, zero proof-hole tokens in code")
    return 0


# ---------------------------------------------------------------------------
# --self-test: fixed snippets, each with the verdict the scanner must give.
# ---------------------------------------------------------------------------

SELF_TEST = [
    # (name, source, expected_hits)
    ("S01 real sorry tactic", "theorem t : True := by\n  sorry\n", 1),
    ("S02 sorryAx term", "macro \"hole\" : tactic => `(tactic| sorryAx _)\n", 1),
    ("S03 mkSorry in metaprogram", "def mint : Expr := mkSorry (Literal.literal \"x\") false\n", 1),
    ("S04 doc comment mention", "/- the gate fails on `sorry` and `sorryAx` -/\ndef ok : Nat := 0\n", 0),
    ("S05 line comment mention", "-- a single `sorry` anywhere breaks the build\ndef ok : Nat := 1\n", 0),
    ("S06 nested block comments", "/- outer /- inner sorry -/ still comment sorry -/\ndef ok : Nat := 2\n", 0),
    ("S07 string literal prose", 'def msg : String := "never write sorry in a proof"\n', 0),
    ("S08 longer identifiers", "def sorrys : Nat := 0\ndef unsorry : Nat := 0\n", 0),
    ("S09 sorry then real code", "theorem a : True := trivial\ntheorem b : True := by sorry\ntheorem c : True := trivial\n", 1),
    ("S10 Audit.lean header shape", "/-\nCEP:WHAT: fails on `sorryAx`, so a single `sorry` anywhere in the\n  development breaks `lake build`.\n-/\nrun_cmd do pure ()\n", 0),
    ("S11 sorryAx doc mention only", "-- `sorryAx` is the axiom a sorry introduces\ndef ok : Nat := 3\n", 0),
]


def run_self_test():
    failures = 0
    for name, src, expected in SELF_TEST:
        got = sum(1 for _ in scan_text(name, src))
        status = "ok" if got == expected else "FAIL"
        if got != expected:
            failures += 1
        print(f"{PROG}: self-test {name}: expected {expected} hit(s), got {got} — {status}")
    if failures:
        print(f"{PROG}: self-test FAILED ({failures} case(s))")
        return 1
    print(f"{PROG}: self-test passed ({len(SELF_TEST)} cases)")
    return 0


def main(argv):
    if len(argv) >= 2 and argv[1] == "--self-test":
        return run_self_test()
    if len(argv) < 2:
        print(f"usage: {PROG} (--self-test | PATH[ PATH...])", file=sys.stderr)
        return 2
    return run_scan(collect_paths(argv[1:]))


if __name__ == "__main__":
    sys.exit(main(sys.argv))
