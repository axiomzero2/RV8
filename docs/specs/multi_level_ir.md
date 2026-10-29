# Multi-Level Intermediate Representation Specification

Status: canonical specification (supplied by project owner, preserved verbatim)
Applies to: crates/rv8_ir and the level-transition lowerings
Cross-references: standard.md 38.17 (IR contract), 38.18 (IR verification), 38.19 (IR determinism)

---

### Abstract

This specification defines the five-level Intermediate Representation (IR) architecture used throughout the compilation pipeline. Each level represents a distinct abstraction of the program, enabling targeted optimizations and transformations. The levels are ordered from high-level semantic representation to low-level machine-specific encoding.

---

## 1. Level 0: High-Level IR (HIR)

### 1.1 Purpose

The HIR preserves the structured control flow and lexical scoping of the source program. It represents the program as a hierarchy of regions, where each region corresponds to a basic block, loop, conditional branch, or scope. This level maintains explicit loop structure and nesting information required for loop transformations.

### 1.2 Structure

The HIR consists of:
- **Regions**: Nested containers representing control flow structures (blocks, loops, conditionals, scopes)
- **Statements**: Individual operations within regions (variable declarations, assignments, expressions, control flow transfers)
- **Expressions**: Computation trees representing values
- **Variables**: Named bindings with type annotations and lifetime information

### 1.3 Invariants

1. The region tree must be acyclic and properly nested.
2. Every loop region must have exactly one header and one latch.
3. Variables declared in a scope region are only accessible within that region and its descendants.
4. All control flow transfers (break, continue, return) must reference valid enclosing regions.

### 1.4 Responsibilities

The HIR serves as the target for structural optimizations that require explicit loop and control flow information. After optimization, the HIR is lowered to the Sea of Nodes representation.

---

## 2. Level 1: Sea of Nodes (MIR)

### 2.1 Purpose

The Sea of Nodes is a data-flow graph representation where nodes represent operations and edges represent data dependencies and control flow. This representation eliminates the distinction between "code" and "data," enabling global optimizations that span across basic block boundaries.

### 2.2 Structure

The Sea of Nodes consists of:
- **Nodes**: Operations (arithmetic, memory access, control flow, function calls)
- **Data Edges**: Directed edges representing value flow from definition to use
- **Control Edges**: Directed edges representing execution ordering
- **Phi Nodes**: Special nodes at control flow merge points that select values based on the incoming control flow path

### 2.3 Invariants

1. Every node must be dominated by its operands (except for loop back-edges).
2. Phi nodes must appear at the beginning of basic blocks where multiple control flow paths merge.
3. The graph must be in Static Single Assignment (SSA) form: every value is defined exactly once.
4. Control flow edges must form a valid control flow graph with a single entry point.

### 2.4 Responsibilities

The Sea of Nodes enables global, machine-independent optimizations such as global value numbering, common subexpression elimination, dead code elimination, and loop-invariant code motion. After optimization, the graph is linearized into the Heterogeneous IR.

---

## 3. Level 2: Heterogeneous IR

### 3.1 Purpose

The Heterogeneous IR is a target-agnostic representation that explicitly models compute, memory, communication, and synchronization operations across diverse hardware architectures (CPUs, GPUs, accelerators). This level abstracts the computational intent without committing to a specific execution model.

### 3.2 Structure

The Heterogeneous IR consists of:
- **Compute Nodes**: Arithmetic, logical, and transcendental operations
- **Memory Nodes**: Load, store, prefetch, and cache management operations
- **Control Nodes**: Branches, loops, function calls
- **Synchronization Nodes**: Barriers, atomics, memory fences
- **Communication Nodes**: Data movement between address spaces (host-device, node-node)
- **Meta Nodes**: Loop bounds, iteration spaces, tiling hints, parallelism annotations

Edges represent:
- **Data Flow**: Value dependencies with type information
- **Control Flow**: Execution ordering
- **Dependency**: RAW, WAR, WAW constraints
- **Spatial Locality**: Memory access patterns (stride, reuse distance)
- **Synchronization**: Ordering constraints
- **Communication**: Data movement between memory hierarchies

### 3.3 Invariants

1. Every memory access must have a corresponding synchronization or dependency edge if it crosses address space boundaries.
2. Communication nodes must specify source and destination address spaces.
3. Synchronization nodes must specify their scope (thread, block, grid, global).
4. The graph must be free of dead nodes (nodes with no uses and no side effects).

### 3.4 Responsibilities

The Heterogeneous IR serves as the input to the autotuner and the backend. It provides a unified representation that can be lowered to any target architecture. The autotuner searches for optimal configurations (pass ordering, iterations, factors) at this level. After autotuning, the IR is lowered to the Low-Level IR.

---

## 4. Level 3: Low-Level IR (LIR)

### 4.1 Purpose

The LIR is a linearized, target-independent representation that uses virtual registers and explicit instruction sequences. This level bridges the gap between the abstract graph representation and the concrete machine instructions.

### 4.2 Structure

The LIR consists of:
- **Basic Blocks**: Linear sequences of instructions with a single entry and single exit
- **Instructions**: Operations with virtual register operands and results
- **Virtual Registers**: Unlimited register identifiers with type and register class annotations
- **Control Flow Graph**: Explicit jumps and branches between basic blocks

### 4.3 Invariants

1. Every virtual register must be defined exactly once (SSA form is relaxed; registers can be redefined).
2. Every instruction must specify the register class of its operands and results.
3. Basic blocks must end with an explicit control flow transfer (jump, branch, return).
4. The control flow graph must be reducible (all loops have a single entry point).

### 4.4 Responsibilities

The LIR is the target for vectorization, instruction selection preparation, and register pressure estimation. It is lowered to the Machine IR by the backend, which maps virtual registers to physical registers and selects target-specific instructions.

---

## 5. Level 4: Machine IR

### 5.1 Purpose

The Machine IR is a target-specific representation that uses physical registers and encodes the exact instruction sequence for the target architecture. This level represents the final form of the program before binary encoding.

### 5.2 Structure

The Machine IR consists of:
- **Basic Blocks**: Linear sequences of machine instructions
- **Instructions**: Target-specific mnemonics with physical register operands
- **Physical Registers**: Actual hardware registers (e.g., RAX, XMM0, thread registers)
- **Memory Operands**: Explicit addressing modes (base + index*scale + displacement)
- **Predicates**: Conditional execution flags (for architectures that support predication)

### 5.3 Invariants

1. Every instruction must be valid for the target instruction set architecture.
2. Physical registers must respect the target's register file constraints (e.g., no mixing of integer and floating-point registers in a single instruction unless explicitly supported).
3. Stack frame layout must conform to the target ABI (alignment, red zone, saved registers).
4. All memory accesses must use valid addressing modes for the target.

### 5.4 Responsibilities

The Machine IR is the output of the backend. It is passed to the binary encoder, which translates the abstract instructions into machine code (binary or assembly). The Machine IR is also used for final peephole optimizations and instruction scheduling to hide latency.

---

## 6. Level Transitions

### 6.1 HIR → Sea of Nodes (Flattening)

The lowering pass converts the region tree into a data-flow graph:
- Each statement becomes a node.
- Control flow edges connect sequential statements.
- Loop headers and latches become back-edges.
- Phi nodes are inserted at control flow merge points.

### 6.2 Sea of Nodes → Heterogeneous IR (Linearization)

The lowering pass converts the graph into a sequence of heterogeneous operations:
- Nodes are topologically sorted.
- Control flow edges become explicit branches.
- Data dependencies are preserved as operand edges.

### 6.3 Heterogeneous IR → LIR (Target Abstraction)

The lowering pass maps abstract operations to target-independent instructions:
- Compute nodes become arithmetic instructions.
- Memory nodes become load/store instructions.
- Communication nodes become explicit data movement instructions.
- Virtual registers are assigned to each value.

### 6.4 LIR → Machine IR (Target Specialization)

The lowering performs instruction selection, register allocation, and scheduling:
- Virtual registers are mapped to physical registers.
- Instructions are selected from the target rule set.
- Instructions are reordered to hide latency.

---

## 7. Integration Points

- **Frontend**: Produces the HIR from the source AST.
- **Borrow Checker**: Operates on the Sea of Nodes to verify memory safety.
- **Autotuner**: Searches for optimal configurations at the Heterogeneous IR level.
- **Backend**: Lowers the Heterogeneous IR through LIR to Machine IR.
- **Binary Encoder**: Translates Machine IR to machine code.
