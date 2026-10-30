# ISL Target Descriptions

## Abstract

ISL is the declarative description language for compiler targets. A description states register lanes per class (names, machine encodings, ABI and clobber roles), per-class move and spill costs, the ABI return and argument registers, and the frame shape. The rv8_isl crate parses and validates descriptions; the allocator core consumes the adapted Plan. Inputs are ISL text; outputs are a validated TargetDescription or an IslError.

## Formal Definitions

A `Lane { name, enc: u16, callee_saved: bool, is_return: bool, args: Vec<u8>, div_clobber: bool }` is one register lane; lane order within a class is the class-local RegId order and `enc` is the machine physical-register encoding. A `Class { name, lanes, move_cost: f64, spill_cost: f64 }` is one register class. A `TargetDescription { name, classes, return_reg, args, frame: Frame }` is the whole target; `Frame { alignment: u32, red_zone_leaf: bool }` is the frame shape.

The grammar, in EBNF:

```text
document  = "target" STRING "{" clause* "}" ;
clause    = class | abi | frame ;
class     = "class" WORD "{" ( cost | lane )* "}" ;
cost      = "move_cost" NUM | "spill_cost" NUM ;
lane      = "lane" WORD "enc" NUM role* ;
role      = "caller-saved" | "callee-saved" | "return"
          | "arg" NUM | "div-clobber" ;
abi       = "abi" ( "return" WORD | "args" WORD+ )+ ;
frame     = "frame" ( "alignment" NUM | "red-zone-leaf" )+ ;
```

## Algorithm

The lexer is a single linear scan preserving byte offsets; '#' starts a line comment. The parser is iterative (no recursion over input): bracket nesting is tracked by an integer under `MAX_ISL_DEPTH`, and documents are bounded by `MAX_ISL_BYTES`. Validation then enforces, in order: non-empty classes, at most 64 lanes per class, strictly increasing machine encodings, ABI roles only in the first class, a resolvable return register with the return role, at most six argument registers each carrying its positional role, and a power-of-two alignment of at least 16. Time is `O(text + lanes)`; space is `O(text)`.

## Invariants

1. A parsed TargetDescription satisfies every checked invariant; consumers never re-validate.
2. Lane encodings are strictly increasing per class, so class-local RegId order is stable.
3. The ABI return and every argument register resolve to lanes of the first class with the matching roles.
4. Costs are strictly positive; the consumer scales them onto the integer grid.
5. Every rejection names the violated invariant; nothing fails silently.

## Integration Points

Depends on: nothing (no external crates).

Depended on by: `rv8_backend::regalloc::desc` (plan_of, x86_64_plan), the CI CEP-lint gate (unrelated), future target tooling.

Signatures: `pub fn parse(src: &str) -> Result<TargetDescription, IslError>`; `pub fn x86_64() -> Result<TargetDescription, IslError>`; `pub const X86_64_ISL: &str`; `pub const TOY4_ISL: &str`.

## Known Limitations

The grammar carries exactly the facts the current allocator and rewriter consume; addressing-mode descriptions, scheduling models, and instruction patterns are future work. The shipped x86-64 description is cross-checked against the hard-coded abi tables lane-for-lane; a target whose encoder is not yet described cannot lower to bytes. Sub-register aliasing (rax/eax/ax/al) is not modeled.
