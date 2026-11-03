/-
CEP:FILE: verification/lean/RV8FV/Spec.lean
CEP:WHAT: Normative syntactic model of the RV8 LIR value language: 64-bit wrapping
  machine words (modulo 2^64, overflow is modeled, never simplified away), virtual
  registers with dense u32-style identity, register/immediate operands, pure and
  effectful instructions, basic blocks with explicit terminators, static size, and
  SSA single-definition well-formedness.
CEP:WHY: CEP&CC 54.4 requires exact model fidelity. Word arithmetic is bounded
  arithmetic on Naturals reduced modulo 2^64, which is operation-for-operation the
  two's-complement machine word. Division by zero raises an explicit trap class,
  matching the fail-closed law (no silent zero).
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: Any Rust LIR transformation that breaks SSA single-definition validity,
  or that assumes unbounded integers, is outside this model and must be rejected by
  the IR verifier.
CEP:ASSUMES: Block identity is the index into the block list; the entry block is 0.
  Shift counts are masked to 6 bits like x86 (b % 64).
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Spec.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: The Rust implementation (crates/rv8_ir::lir and rv8_backend) is
  linked to this model by runtime bridges: the brute-force regalloc corpora and the
  end-to-end deterministic suites execute the same fixtures this model encodes in
  RV8FV/Smoke.lean. Verified extraction is future work.
CEP:FV-AXIOMS: none.
-/

namespace RV8FV

/-! ## Machine words: bounded 64-bit arithmetic, overflow modeled. -/

/-- 2^64, the word modulus. -/
def W64 : Nat := 18446744073709551616

/-- A machine word is a natural number; every operation below reduces modulo `W64`.
    An `abbrev` so all `Nat` instances (decidable equality, arithmetic) apply. -/
abbrev Word := Nat

/-- Wrap a natural into the 64-bit domain. All arithmetic goes through this. -/
def wrap (n : Nat) : Word := n % W64

/-- Two's-complement wrap-around addition. Overflow wraps, as on hardware. -/
def wadd (a b : Word) : Word := wrap (a + b)

/-- Two's-complement wrap-around subtraction: `(a - b) mod 2^64`. -/
def wsub (a b : Word) : Word := wrap (a + W64 - b)

/-- Two's-complement wrap-around multiplication. -/
def wmul (a b : Word) : Word := wrap (a * b)

/-- Bitwise and/or/xor on 64-bit words; results stay below 2^64 by `Nat.land/lor/lxor`. -/
def wand (a b : Word) : Word := Nat.land a b
def wor  (a b : Word) : Word := Nat.lor a b
def wxor (a b : Word) : Word := Nat.xor a b

/-- Logical shift left with the count masked to 6 bits (x86 `shl` semantics),
    then wrapped into the 64-bit domain. -/
def wshl (a b : Word) : Word := wrap (a * 2 ^ (b % 64))

/-- Logical shift right with the count masked to 6 bits (x86 `shr` semantics). -/
def wlshr (a b : Word) : Word := a / 2 ^ (b % 64)

/-! ## Binary operations and the division trap. -/

/-- The nine value-space binary operations of the LIR. -/
inductive Binop where
  | add | sub | mul | udiv | and | or | xor | shl | lshr
  deriving DecidableEq, Repr

/-- Operation semantics on words. `udiv` by zero is `none`: the machine traps.
    Every other operation is total and wrapping. This is the fail-closed contract. -/
def Binop.eval (o : Binop) (a b : Word) : Option Word :=
  match o with
  | .add  => some (wadd a b)
  | .sub  => some (wsub a b)
  | .mul  => some (wmul a b)
  | .udiv => if b = 0 then none else some (a / b)
  | .and  => some (wand a b)
  | .or   => some (wor a b)
  | .xor  => some (wxor a b)
  | .shl  => some (wshl a b)
  | .lshr => some (wlshr a b)

/-- Constant evaluation exposed to the folding pass: identical to `Binop.eval`. -/
def foldEval (o : Binop) (a b : Word) : Option Word := o.eval a b

/-! ## Operands, instructions, terminators, programs. -/

/-- Virtual register identity: a dense natural index (IR contract 38.17). -/
abbrev VReg := Nat

/-- Block identity: index into the block list. -/
abbrev BlockId := Nat

/-- An operand is either a virtual register read or an immediate word. -/
inductive Arg where
  | reg : VReg → Arg
  | imm : Word → Arg
  deriving DecidableEq, Repr

/-- One instruction of a basic block. `copy d (imm w)` is the immediate load
    (catalog: LoadImm); `copy d (reg s)` is a register-to-register move. -/
inductive Instr where
  | nop : Instr                                        -- placeholder emitted by DCE
  | copy : VReg → Arg → Instr                          -- d := arg
  | binop : VReg → Binop → Arg → Arg → Instr           -- d := a op b
  | load : VReg → Arg → Instr                          -- d := mem[addr]
  | store : Arg → Arg → Instr                          -- mem[addr] := val
  | call : VReg → Nat → List Arg → Instr               -- d := opaque call, clobbers
  deriving DecidableEq, Repr

/-- Block terminator. Conditional branch reads a word and takes the `taken`
    target when it is non-zero, else the fallthrough (x86 `jnz` shape). -/
inductive Term where
  | jmp : BlockId → Term
  | br : Arg → BlockId → BlockId → Term
  | ret : Option Arg → Term
  deriving DecidableEq, Repr

/-- A basic block: straight-line instructions followed by one terminator. -/
structure Block where
  instrs : List Instr
  term : Term
  deriving Repr, DecidableEq

/-- A program: a list of blocks; entry is block 0. -/
structure Prog where
  blocks : List Block
  deriving Repr, DecidableEq

/-- The vreg defined by an instruction, if any (`store` and `nop` define none). -/
def Instr.defOf : Instr → Option VReg
  | .nop => none
  | .copy d _ => some d
  | .binop d _ _ _ => some d
  | .load d _ => some d
  | .store _ _ => none
  | .call d _ _ => some d

/-- The registers an operand reads: none for an immediate. -/
def Arg.regsOf : Arg → List VReg
  | .imm _ => []
  | .reg r => [r]

/-- The vregs read by an instruction (in terms of `Arg.regsOf`). -/
def Instr.usesOf : Instr → List VReg
  | .nop => []
  | .copy _ a => a.regsOf
  | .binop _ _ a b => a.regsOf ++ b.regsOf
  | .load _ a => a.regsOf
  | .store a b => a.regsOf ++ b.regsOf
  | .call _ _ args => args.flatMap Arg.regsOf

/-- The vregs read by a terminator (in terms of `Arg.regsOf`). -/
def Term.usesOf : Term → List VReg
  | .jmp _ => []
  | .br a _ _ => a.regsOf
  | .ret a => match a with
    | none => []
    | some c => c.regsOf

/-- Whether an instruction is free of memory effects and of traps other than the
    division trap. Only such instructions may ever be removed or introduced by a
    proved pass. `nop` is deliberately not pure so the DCE fixpoint terminates. -/
def Instr.pure : Instr → Bool
  | .nop => false
  | .copy _ _ => true
  | .binop _ o _ _ => o ≠ .udiv
  | .load _ _ => false
  | .store _ _ => false
  | .call _ _ _ => false

/-- Static size: total instruction count. Terminators are not counted. -/
def Prog.size (p : Prog) : Nat :=
  p.blocks.foldl (fun acc bl => acc + bl.instrs.length) 0

/-- Number of non-nop instructions: the DCE progress measure. -/
def Prog.countNn (p : Prog) : Nat :=
  p.blocks.foldl (fun acc bl =>
    acc + bl.instrs.countP (fun i => match i with | .nop => false | _ => true)) 0

/-! ## SSA well-formedness (single definition per vreg). -/

/-- Number of definitions of `v` in the whole program. -/
def Prog.countDefs (p : Prog) (v : VReg) : Nat :=
  p.blocks.foldl (fun acc bl =>
    acc + bl.instrs.countP (fun i => i.defOf = some v)) 0

/-- SSA single-definition well-formedness. All proved passes require it, and every
    in-place rewrite in this development preserves it. -/
def WfSSA (p : Prog) : Prop := ∀ v, p.countDefs v ≤ 1

end RV8FV
