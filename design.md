
### 1. Control Flow & Structural Optimizations
*Focus: Manipulating the shape of the program and loop structures.*

1.  **Basic Block Reordering:** Aligning hot paths for branch prediction.
2.  **Loop Unrolling:** Replicating loop bodies to reduce overhead.
3.  **Loop Fusion:** Merging adjacent loops to improve locality.
4.  **Loop Fission:** Splitting large loops to reduce register pressure.
5.  **Loop Tiling (Blocking):** Optimizing for cache hierarchy.
6.  **Loop Interchange:** Swapping nested loops for better access patterns.
7.  **Loop Skewing:** Breaking dependencies to enable parallelism.
8.  **Loop Peeling:** Handling first/last iterations separately.
9.  **Loop Versioning:** Creating specialized versions for different conditions.
10. **Loop Unswitching:** Moving conditionals out of loops.
11. **Loop Invariant Code Motion (LICM):** Hoisting calculations out of loops.
12. **Induction Variable Simplification:** Optimizing loop counters.
13. **Strength Reduction in Loops:** Replacing expensive ops with cheaper ones.
14. **Dead Loop Elimination:** Removing loops that do nothing.
15. **Loop Rotation:** Moving the latch to the top/bottom for better pipelining.
16. **Software Pipelining:** Overlapping iterations for throughput.
17. **Global Code Motion (GCM):** Moving instructions to optimal basic blocks.
18. **Tail Duplication:** Duplicating tails to simplify control flow.
19. **Jump Threading:** Redirecting jumps through intermediate blocks.
20. **Conditional Branch Optimization:** Simplifying complex branch logic.
21. **Switch Statement Lowering:** Using jump tables or binary search.
22. **CFG Flattening:** Linearizing control flow for specific hardware/VMs.
23. **Irreducible CFG Reduction:** Transforming complex graphs into reducible ones.
24. **Edge Splitting:** Breaking critical edges for SSA construction.
25. **Critical Edge Splitting:** Specific handling for Phi node placement.
26. **Dominator Tree Updates:** Maintaining dominance info during transforms.
27. **Post-Dominator Analysis:** Finding nodes that must execute after a point.
28. **Natural Loop Detection:** Identifying canonical loop structures.
29. **Reducibility Testing:** Checking if CFG can be reduced to a single node.
30. **Control Dependence Graph Construction:** Tracking conditional execution.

### 2. Data Flow & Value-Based Optimizations
*Focus: Tracking how data moves, changes, and is used.*

31. **Global Value Numbering (GVN):** Identifying identical computations.
32. **Sparse Conditional Constant Propagation (SCCP):** Propagating constants along feasible paths.
33. **Dead Store Elimination:** Removing writes that are never read.
34. **Dead Code Elimination (DCE):** Removing unused instructions.
35. **Common Subexpression Elimination (CSE):** Merging duplicate calculations.
36. **Partial Redundancy Elimination (PRE):** Eliminating computations on some paths.
37. **Lazy Code Motion:** Placing computations as late as possible.
38. **Copy Propagation:** Replacing variables with their source values.
39. **Constant Folding:** Computing constant expressions at compile time.
40. **Constant Propagation:** Spreading known constant values.
41. **Range Analysis:** Tracking min/max values of variables.
42. **Interval Analysis:** A more robust form of range tracking.
43. **Affine Analysis:** Modeling relationships for loop bounds (kept for analysis, not full polyhedral).
44. **Points-to Analysis:** Determining what memory locations pointers refer to.
45. **Steensgaard’s Alias Analysis:** Fast, conservative alias tracking.
46. **Andersen’s Alias Analysis:** More precise, slower alias tracking.
47. **Escape Analysis:** Determining if objects leave their scope.
48. **Scalar Replacement of Aggregates (SRA):** Breaking structs into scalars.
49. **Memory SSA Construction:** Tracking memory state in SSA form.
50. **Phi Node Optimization:** Simplifying redundant Phi nodes.
51. **Redundant Phi Elimination:** Removing Phis that just pass through values.
52. **Critical Edge Phi Insertion:** Adding Phis where needed for correctness.
53. **Value Range Propagation (VRP):** Using ranges to optimize branches/checks.
54. **Bitwise Range Analysis:** Tracking known bits (0s and 1s).
55. **Symbolic Execution for Constants:** Solving for constants using logic.
56. **Taint Analysis:** Tracking data origin for security/side-effects.
57. **Liveness Analysis:** Determining when values are needed.
58. **Reaching Definitions:** Tracking where values come from.
59. **Available Expressions:** Tracking which computations are already done.
60. **Very Busy Expressions:** Tracking computations that will definitely happen.
61. **Anticipatable Expressions:** Tracking computations that might happen.
62. **Safe Points Insertion:** Adding hooks for GC or interrupts.
63. **Null Check Elimination:** Removing redundant null checks.
64. **Bounds Check Elimination:** Removing array bounds checks via analysis.
65. **Divide-by-Zero Protection Removal:** Removing checks proven safe.
66. **Overflow Check Elimination:** Removing checks via range analysis.
67. **Signed/Unsigned Conversion Optimization:** Simplifying casts.
68. **Zero/Sign Extension Elimination:** Removing redundant extensions.
69. **Truncation Elimination:** Removing redundant truncations.
70. **Type Punning Optimization:** Optimizing safe type reinterpretations.

### 3. Memory & Cache Hierarchy Optimizations
*Focus: Maximizing memory throughput and minimizing latency.*

71. **Cache-Aware Tiling:** Optimizing data access for cache lines.
72. **Cache-Oblivious Algorithms:** Using recursive decomposition for locality.
73. **Data Prefetching Insertion:** Adding software prefetch instructions.
74. **Stream Buffer Optimization:** Aligning data for hardware streamers.
75. **Array Padding:** Preventing cache line conflicts (false sharing).
76. **Structure Splitting:** Breaking structs for better access patterns.
77. **Field Reordering:** Packing fields for alignment and locality.
78. **Stack Color Allocation:** Reusing stack slots efficiently.
79. **Heap Allocation Elision:** Allocating on the stack instead.
80. **Object Inlining:** Embedding small objects directly.
81. **Write Barrier Elision:** Removing GC barriers when safe.
82. **Read Barrier Elision:** Removing GC read barriers when safe.
83. **NUMA-Aware Data Placement:** Placing data near executing cores.
84. **TLB Shootdown Minimization:** Reducing page table invalidation costs.
85. **Page Coloring:** Optimizing virtual-to-physical mapping.
86. **Memory Coalescing:** Combining adjacent memory accesses (GPU).
87. **Shared Memory Tiling:** Using fast local memory (GPU).
88. **Register Spilling Optimization:** Minimizing spills to memory.
89. **Live Range Splitting:** Breaking live ranges to reduce pressure.
90. **Rematerialization:** Recomputing values instead of loading them.
91. **Pointer Swizzling:** Converting pointers to offsets for locality.
92. **Compact Data Layouts:** Minimizing memory footprint.
93. **Bitpacking:** Storing multiple small values in one word.
94. **Delta Encoding:** Storing differences instead of absolute values.
95. **Memory Aliasing Disambiguation:** Proving pointers don't overlap.

### 4. Parallelism & Vectorization
*Focus: Exploiting SIMD and multi-core hardware.*

96. **Auto-Vectorization (SLP):** Packing scalar ops into SIMD.
97. **Loop Vectorization:** Converting loops to SIMD operations.
98. **SIMD Width Selection:** Choosing best vector width (128/256/512).
99. **Masked Vector Operations:** Handling conditional SIMD safely.
100. **Gather/Scatter Optimization:** Optimizing non-contiguous access.
101. **Thread-Level Parallelism (TLP) Extraction:** Identifying parallel tasks.
102. **Task Decomposition:** Breaking work into parallel chunks.
103. **Work Stealing Schedule Optimization:** Balancing load dynamically.
104. **OpenMP Directive Lowering:** Implementing parallel pragmas.
105. **MPI Call Aggregation:** Combining small messages.
106. **Reduction Variable Optimization:** Parallelizing sums/products.
107. **Private Variable Optimization:** Managing thread-local state.
108. **Barrier Elimination:** Removing unnecessary synchronization.
109. **Lock Elision:** Using hardware transactions instead of locks.
110. **Atomic Operation Lowering:** Mapping atomics to ISA primitives.
111. **False Sharing Prevention:** Padding data to avoid cache contention.
112. **Core Affinity Setting:** Pinning threads to specific cores.
113. **Hyperthreading Awareness:** Optimizing for SMT resources.
114. **GPU Kernel Fusion:** Merging GPU kernels to reduce launches.
115. **Warp Divergence Minimization:** Reducing branching in GPU warps.
116. **Occupancy Optimization:** Maximizing active GPU warps.
117. **Cooperative Group Optimization:** Using advanced GPU sync groups.
118. **Tensor Core Mapping:** Using specialized AI hardware units.
119. **Sparse Matrix Vectorization:** Optimizing for sparse data.
120. **Bit-Level Parallelism:** Using bitwise ops for parallel logic.

### 5. Procedure & Link-Time Optimizations
*Focus: Crossing function and module boundaries.*

121. **Inlining:** Replacing calls with function bodies.
122. **Devirtualization:** Resolving virtual calls to direct calls.
123. **Whole-Program Devirtualization:** Resolving calls across modules.
124. **Clone Specialization:** Creating specialized function copies.
125. **Function Outlining:** Extracting cold code to reduce size.
126. **Tail Call Optimization:** Reusing stack frames for tail calls.
127. **Sibling Call Optimization:** Optimizing calls between siblings.
128. **Argument Promotion:** Passing values in registers instead of memory.
129. **Return Value Optimization (RVO):** Eliminating temporary returns.
130. **Named Return Value Optimization (NRVO):** Extending RVO to named vars.
131. **Link-Time Optimization (LTO):** Optimizing across all object files.
132. **ThinLTO:** Scalable distributed LTO.
133. **Interprocedural Constant Propagation:** Propagating constants across calls.
134. **Interprocedural Dead Code Elimination:** Removing unused functions.
135. **Global Inline Assembly Optimization:** Optimizing around asm blocks.
136. **Import/Export Pruning:** Removing unused symbols.
137. **Section Garbage Collection:** Removing unused code sections.
138. **ICF (Identical Code Folding):** Merging identical functions.
139. **COMDAT Folding:** Merging duplicate template instantiations.
140. **Profile-Guided Inlining:** Using runtime data to decide inlining.

### 6. Advanced & Systems Optimizations
*Focus: The "Compiler 2.0" features, ML, and systems integration.*

141. **Partial Evaluation (PE):** Specializing code with known inputs.
142. **Supercompilation:** Driving evaluation by context.
143. **Deforestation:** Eliminating intermediate lists/structures.
144. **Short-cut Fusion:** Fusing producer/consumer operations.
145. **Church-Rosser Property Verification:** Ensuring rewrite confluence.
146. **Dependence Graph Optimization:** Optimizing based on data deps.
147. **Multi-Versioning:** Creating multiple code paths for runtime dispatch.
148. **Profile-Guided Optimization (PGO):** Using profiling data for optimization.
149. **Feedback-Directed Optimization (FDO):** Continuous improvement from production.
150. **Machine Learning Guided Heuristics:** Using ML to pick optimization strategies.
151. **Bayesian Optimization for Flags:** Efficiently tuning compiler parameters.
152. **Energy-Aware Scheduling:** Optimizing for power efficiency.
153. **Thermal-Aware Compilation:** Avoiding hotspots in code layout.
154. **Reliability-Aware Compilation:** Mitigating soft errors.
155. **Security-Hardening Passes:** CFI, Stack Canaries, etc.
156. **Watermarking:** Embedding ownership info in code.
157. **Compile-Time Caching:** Saving results to speed up rebuilds.
158. **Incremental Compilation:** Only recompiling changed parts.
159. **Parallel Compilation:** Using multiple cores to compile.
160. **Distributed Compilation:** Using a cluster to compile.
161. **Cloud-Based Offloading:** Offloading heavy tuning to the cloud.
162. **AOT Compilation Optimization:** Optimizing for static binaries.
163. **Tiered Compilation:** Using different optimization levels for different phases.
164. **Inline Caching:** Caching method lookups at runtime.
165. **Polymorphic Inline Caches (PIC):** Handling multiple types efficiently.
166. **Megamorphic Dispatch Handling:** Optimizing highly dynamic dispatch.
167. **Hidden Class Optimization:** Tracking object shapes for speed.
168. **Shape Analysis:** Analyzing heap structure for optimization.
169. **Region-Based Memory Management:** Managing memory in regions.
170. **Capability-Based Security Optimization:** Enforcing security via types.
171. **Effect-Typed Optimization:** Optimizing based on side-effect info.
172. **Linear Type Optimization:** Optimizing single-use resources.
173. **Session Type Optimization:** Optimizing communication protocols.
174. **Refinement Type Optimization:** Using precise types for optimization.
175. **Liquid Types Integration:** Using logical predicates for types.
176. **Dependent Type Erasure:** Removing types after checking.

---

### Summary of Changes
*   **Removed:** 24 items related to excessive complexity, research-only concepts (E-graphs, Polyhedral), or irrelevant domains (Quantum, JIT).
*   **Kept:** 176 high-impact optimizations covering everything from basic loop transforms to advanced ML-guided autotuning and semantic analysis.


## Min-Cost Multi-Commodity Flow Register Allocation: Complete Technical Design

### 1. Mathematical Formulation

The register allocation problem is formulated as a **Min-Cost Multi-Commodity Flow (MCF)** problem on a directed graph $G = (V, E)$.

**Given:**
- A set of program points $P = \{p_1, p_2, \ldots, p_n\}$ (basic block boundaries, loop headers, instruction boundaries)
- A set of live ranges $L = \{l_1, l_2, \ldots, l_m\}$, where each $l_i = (v_i, start_i, end_i, type_i)$ represents a value $v_i$ that is live from $start_i$ to $end_i$ with register type $type_i$
- A set of register classes $R = \{r_1, r_2, \ldots, r_k\}$ (e.g., GPR, FPR, vector, predicate)
- For each register class $r_j$, a capacity $C_j$ (number of physical registers available)
- A cost function $cost: E \times R \rightarrow \mathbb{R}^+$ representing the cost of using a register or spilling

**Objective:**
Find a flow assignment $f: E \times R \rightarrow \mathbb{R}^+$ that minimizes:
$$\min \sum_{e \in E} \sum_{r \in R} f(e, r) \cdot cost(e, r)$$

**Subject to:**
1. **Flow conservation:** For each node $v \in V$ and register class $r \in R$:
   $$\sum_{e \in \delta^+(v)} f(e, r) - \sum_{e \in \delta^-(v)} f(e, r) = b(v, r)$$
   where $b(v, r)$ is the supply/demand at node $v$ for commodity $r$

2. **Capacity constraints:** For each edge $e \in E$ and register class $r \in R$:
   $$0 \leq f(e, r) \leq u(e, r)$$
   where $u(e, r)$ is the capacity of edge $e$ for commodity $r$

3. **Multi-commodity coupling:** For nodes where type conversions occur (e.g., int-to-float), the flow of different commodities must satisfy conversion constraints

---

### 2. Network Construction

The flow network is constructed from the LIR (Low-level Intermediate Representation) as follows:

#### 2.1 Node Construction

**Program Point Nodes:**
For each program point $p_i \in P$, create a node $n_i \in V$.

**Value Nodes:**
For each live range $l_j = (v_j, start_j, end_j, type_j)$, create:
- A **source node** $s_j$ at $start_j$ with supply $b(s_j, type_j) = +1$
- A **sink node** $t_j$ at $end_j$ with demand $b(t_j, type_j) = -1$

**Spill Nodes:**
For each program point $p_i$, create a **spill node** $sp_i$ that represents the stack/memory. This node has:
- Infinite capacity edges to/from all value nodes
- High cost edges (representing memory access latency)

**Register Alias Nodes:**
For fixed registers (e.g., x86 `rax` for division results, stack pointer), create alias nodes that force certain values to flow through specific registers.

#### 2.2 Edge Construction

**Register Edges:**
For each program point $p_i$ to $p_{i+1}$ (consecutive points in control flow order), create edges:
- For each register class $r_j$: an edge $(n_i, n_{i+1})$ with capacity $C_j$ and cost $0$
- These edges represent the availability of physical registers across program points

**Spill Edges:**
For each value node $v_j$:
- An edge $(v_j, sp_{start_j})$ with capacity $\infty$ and cost $cost_{spill\_store}$
- An edge $(sp_{end_j}, v_j)$ with capacity $\infty$ and cost $cost_{spill\_load}$
- These edges represent spilling the value to memory at its definition and reloading at its use

**Move Edges:**
For values that are copied (e.g., `a = b`):
- An edge $(v_b, v_a)$ with capacity $1$ and cost $cost_{move}$
- This encourages the allocator to assign the same register to both (register coalescing)

**Conversion Edges:**
For type conversions (e.g., int-to-float):
- An edge $(v_{int}, v_{float})$ with capacity $1$ and cost $cost_{conversion}$
- This models the instruction that converts between register classes

**Fixed Register Edges:**
For values that must use specific registers:
- An edge $(v_j, alias_{reg})$ with capacity $1$ and cost $0$
- An edge $(alias_{reg}, v_j)$ with capacity $1$ and cost $0$
- This forces the value to flow through the aliased register

#### 2.3 Cost Assignment

The cost function encodes the optimization objectives:

**Register Usage Cost:**
$$cost(register\_edge) = 0$$
Using a physical register is free (the capacity constraint handles scarcity).

**Spill Cost:**
$$cost(spill\_edge) = latency_{memory} + overhead_{spill\_code}$$
Spilling is expensive. The cost includes:
- Memory access latency (L1 cache: ~4 cycles, L2: ~12 cycles, memory: ~200 cycles)
- Instruction overhead (additional load/store instructions)

**Move Cost:**
$$cost(move\_edge) = latency_{register\_to\_register}$$
Moving between registers is cheap but not free (1 cycle typically).

**Conversion Cost:**
$$cost(conversion\_edge) = latency_{conversion\_instruction}$$
Type conversions have instruction-specific latencies.

**Preference Cost:**
For targets with register preferences (e.g., x86 prefers `rax` for certain operations):
$$cost(preferred\_register\_edge) = 0$$
$$cost(non\_preferred\_register\_edge) = \epsilon$$
A small penalty $\epsilon$ encourages using preferred registers.

---

### 3. Algorithm: Successive Shortest Path with Potentials

We use the **Successive Shortest Path (SSP)** algorithm with **Johnson's potentials** for efficiency.

#### 3.1 Algorithm Overview

1. **Initialize:** Start with zero flow on all edges.
2. **Find shortest path:** For each unsatisfied demand (sink node with $b < 0$), find the shortest path from a supply node to this demand node using Dijkstra's algorithm with potentials.
3. **Augment flow:** Push one unit of flow along the shortest path.
4. **Update potentials:** Update the potentials to maintain non-negative edge weights for the next iteration.
5. **Repeat:** Until all demands are satisfied.

#### 3.2 Why SSP with Potentials?

- **Correctness:** SSP guarantees an optimal solution for MCF problems with integer capacities (which ours has).
- **Efficiency:** Johnson's potentials allow us to use Dijkstra's algorithm (which is fast with a binary heap) instead of Bellman-Ford (which is slower).
- **Incrementality:** SSP can be restarted from a partial solution, which is useful for incremental compilation.

#### 3.3 Pseudocode

```
function solve_mcf(G, b, u, cost):
    f = 0  // zero flow
    pi = 0  // zero potentials
    
    while exists node v with b(v) < 0:
        // Find shortest path from any supply node to v
        (path, dist) = dijkstra_with_potentials(G, cost, pi, v)
        
        if path is None:
            error "Infeasible flow problem"
        
        // Augment flow along path
        for edge e in path:
            f(e) += 1
        
        // Update supplies/demands
        for node n in path:
            b(n) -= 1  // supply node
            b(n) += 1  // demand node
        
        // Update potentials
        for node n in G:
            pi(n) += dist(n)
    
    return f
```

#### 3.4 Dijkstra with Potentials

```
function dijkstra_with_potentials(G, cost, pi, target):
    // Reduced cost: cost'(u,v) = cost(u,v) + pi(u) - pi(v)
    // This ensures cost' >= 0 for all edges
    
    dist = infinity for all nodes
    dist[target] = 0
    prev = None for all nodes
    pq = priority queue with (0, target)
    
    while pq is not empty:
        (d, u) = pq.extract_min()
        
        if d > dist(u):
            continue
        
        for edge (u, v) in G:
            reduced_cost = cost(u, v) + pi(u) - pi(v)
            if dist(u) + reduced_cost < dist(v):
                dist(v) = dist(u) + reduced_cost
                prev(v) = u
                pq.insert((dist(v), v))
    
    return (reconstruct_path(prev, target), dist)
```

#### 3.5 Multi-Commodity Handling

For multiple commodities (register classes), we solve each commodity independently **except** at conversion nodes where commodities interact.

**Decomposition Strategy:**
1. Solve GPR commodity first (most constrained)
2. Solve FPR commodity next
3. Solve vector commodity last
4. At conversion nodes, add coupling constraints: if a value is converted from GPR to FPR, the GPR flow must end at the conversion node and the FPR flow must start there.

**Coupling Constraint Enforcement:**
When solving commodity $r_2$ (e.g., FPR), if a conversion node requires input from commodity $r_1$ (e.g., GPR), we:
1. Check if the GPR flow has been assigned to the conversion node.
2. If yes, add a fixed supply at the conversion node for commodity $r_2$.
3. If no, mark the conversion as infeasible and backtrack (or use a heuristic to resolve).

---

### 4. Integration with LIR

#### 4.1 Live Range Analysis

Before constructing the flow network, we perform **liveness analysis** on the LIR:

1. **Compute live-in and live-out sets** for each basic block using backward dataflow analysis.
2. **Compute live ranges** for each value: the set of program points where the value is live.
3. **Identify interference:** Two values interfere if their live ranges overlap.

**LIR Extensions:**
The LIR must carry additional metadata:
- `live_range(value)`: the set of program points where the value is live
- `def_point(value)`: the program point where the value is defined
- `use_points(value)`: the set of program points where the value is used
- `register_class(value)`: the required register class for the value

#### 4.2 Network Construction from LIR

```
function build_flow_network(lir):
    G = empty graph
    
    // Create program point nodes
    for block in lir.blocks:
        for instr in block.instructions:
            create_node(G, instr.program_point)
    
    // Create value nodes
    for value in lir.values:
        create_node(G, value.source_node)
        create_node(G, value.sink_node)
        
        // Add supply/demand
        set_supply(G, value.source_node, value.register_class, +1)
        set_demand(G, value.sink_node, value.register_class, -1)
    
    // Create register edges
    for consecutive program points (p1, p2):
        for reg_class in register_classes:
            add_edge(G, p1, p2, capacity=reg_class.capacity, cost=0)
    
    // Create spill edges
    for value in lir.values:
        add_edge(G, value.source_node, spill_node(value.def_point), 
                 capacity=infinity, cost=spill_store_cost)
        add_edge(G, spill_node(value.use_point), value.sink_node,
                 capacity=infinity, cost=spill_load_cost)
    
    // Create move edges
    for instr in lir.instructions:
        if instr.opcode == MOVE:
            add_edge(G, instr.src_value, instr.dst_value, 
                     capacity=1, cost=move_cost)
    
    // Create conversion edges
    for instr in lir.instructions:
        if instr.opcode == CONVERT:
            add_edge(G, instr.src_value, instr.dst_value,
                     capacity=1, cost=conversion_cost)
    
    return G
```

#### 4.3 Extracting Register Assignments

After solving the MCF, we extract register assignments from the flow:

```
function extract_assignments(flow, G):
    assignments = {}
    
    for edge in G.edges:
        if edge.is_register_edge and flow(edge) > 0:
            // This edge carries flow, meaning a value is assigned to this register
            value = edge.source_node.value
            register = edge.register_index
            assignments[value] = register
    
    return assignments
```

#### 4.4 Spill Code Generation

If the flow uses spill edges, we insert spill code into the LIR:

```
function insert_spill_code(lir, flow, G):
    for edge in G.edges:
        if edge.is_spill_edge and flow(edge) > 0:
            if edge.is_spill_store:
                // Insert store instruction at the definition point
                insert_instruction(lir, edge.program_point, 
                                   STORE(edge.value, stack_slot(edge.value)))
            else:
                // Insert load instruction at the use point
                insert_instruction(lir, edge.program_point,
                                   LOAD(edge.value, stack_slot(edge.value)))
```

---

### 5. Heterogeneous Register Classes

#### 5.1 Register Class Definition

Each target defines its register classes:

```rust
pub struct RegisterClass {
    pub name: String,
    pub registers: Vec<PhysicalRegister>,
    pub capacity: usize,
    pub spill_cost: f64,
    pub move_cost: f64,
}

pub struct PhysicalRegister {
    pub id: usize,
    pub name: String,
    pub is_fixed: bool,  // e.g., stack pointer, return value register
    pub aliases: Vec<usize>,  // e.g., x86 rax aliases eax, ax, al
}
```

**Example: x86-64 Register Classes**

```rust
let gpr = RegisterClass {
    name: "GPR".to_string(),
    registers: vec![
        PhysicalRegister { id: 0, name: "rax".to_string(), is_fixed: false, aliases: vec![8, 12, 16] },
        PhysicalRegister { id: 1, name: "rbx".to_string(), is_fixed: false, aliases: vec![9, 13, 17] },
        PhysicalRegister { id: 2, name: "rcx".to_string(), is_fixed: false, aliases: vec![10, 14, 18] },
        // ... rdx, rsi, rdi, rbp, r8-r15
    ],
    capacity: 16,  // 16 general-purpose registers
    spill_cost: 10.0,
    move_cost: 1.0,
};

let fpr = RegisterClass {
    name: "FPR".to_string(),
    registers: vec![
        PhysicalRegister { id: 0, name: "xmm0".to_string(), is_fixed: false, aliases: vec![] },
        // ... xmm1-xmm15
    ],
    capacity: 16,
    spill_cost: 10.0,
    move_cost: 1.0,
};

let vector = RegisterClass {
    name: "VECTOR".to_string(),
    registers: vec![
        PhysicalRegister { id: 0, name: "ymm0".to_string(), is_fixed: false, aliases: vec![16] },  // aliases xmm0
        // ... ymm1-ymm15
    ],
    capacity: 16,
    spill_cost: 15.0,  // wider registers are more expensive to spill
    move_cost: 2.0,
};
```

#### 5.2 Register Class Interactions

**Aliasing:** When a value is assigned to `rax`, it also occupies `eax`, `ax`, and `al`. The flow network must model this:
- Create edges between alias registers with capacity 1 and cost 0.
- When flow passes through `rax`, it blocks flow through `eax`, `ax`, `al`.

**Sub-register Assignment:** If a value only needs 32 bits, it can be assigned to `eax` instead of `rax`. The flow network prefers shorter registers (lower cost) to leave longer registers available for 64-bit values.

**Type Conversion:** When converting from GPR to FPR (e.g., `cvtsi2sd`), the flow network:
1. Ends the GPR flow at the conversion instruction.
2. Starts the FPR flow at the conversion instruction.
3. Adds a conversion edge with cost = instruction latency.

---

### 6. Concurrency Model

#### 6.1 Function-Level Parallelism

Register allocation is performed independently for each function. The work-stealing scheduler distributes functions to worker threads:

```rust
pub fn allocate_registers(lir: &LirFunction, target: &TargetIsa) -> RegisterAllocation {
    // Build flow network (thread-local, no synchronization)
    let network = build_flow_network(lir);
    
    // Solve MCF (thread-local computation)
    let flow = solve_mcf(network);
    
    // Extract assignments (thread-local)
    let assignments = extract_assignments(flow, network);
    
    // Insert spill code (thread-local LIR mutation)
    insert_spill_code(lir, flow, network);
    
    RegisterAllocation { assignments }
}
```

#### 6.2 Parallel Shortest Path (Optional)

For very large functions, we can parallelize the Dijkstra searches within a single MCF solve:

**Delta-Stepping Algorithm:**
Instead of a single-threaded Dijkstra, use the delta-stepping algorithm which can process multiple nodes in parallel if they have similar distances.

**Bucket-Based Parallelism:**
Divide nodes into buckets by distance. Process all nodes in a bucket in parallel before moving to the next bucket.

---

### 7. Performance Optimizations

#### 7.1 Sparse Graph Representation

The flow network is sparse (each node has only a few edges). Use adjacency lists instead of adjacency matrices:

```rust
pub struct FlowNetwork {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub adjacency: Vec<Vec<usize>>,  // node index -> list of edge indices
}
```

#### 7.2 Incremental Solving

For incremental compilation (when only part of the function changes), reuse the previous flow solution:
1. Identify changed live ranges.
2. Remove their flow from the network.
3. Re-solve only the affected portion.

#### 7.3 Region-Based Allocation

For very large functions, decompose into smaller regions (e.g., basic blocks or loop nests) and solve each region independently. Connect regions with "boundary" nodes that represent values live across region boundaries.

#### 7.4 Heuristic Initialization

Before running MCF, use a fast heuristic (e.g., linear scan) to get an initial feasible solution. Then run MCF to optimize it. This reduces the number of augmenting paths needed.

---

### 8. Edge Cases and Correctness

#### 8.1 Infeasible Problems

If the MCF solver cannot find a feasible flow (e.g., not enough registers even with spilling), the problem is infeasible. This should never happen if we have spill edges with infinite capacity.

**Debugging:** If infeasibility occurs, it indicates a bug in the network construction (e.g., missing spill edges, incorrect supply/demand).

#### 8.2 Cycles in the Flow

The MCF solution may contain cycles (flow that goes in a loop). These cycles do not affect the cost (they add zero net flow) but waste computation.

**Cycle Removal:** After solving, remove any cycles by finding and canceling them. This is a post-processing step.

#### 8.3 Integer vs. Fractional Flow

The SSP algorithm guarantees integer flow if all capacities and supplies are integers (which ours are). This is critical because we cannot assign "half a register."

#### 8.4 Tie-Breaking

If multiple paths have the same cost, the solver must break ties consistently. Use a deterministic tie-breaking rule (e.g., prefer lower register indices) to ensure reproducible results.

---

### 9. Complete Backend Integration

The register allocator is one component of the backend. The full backend pipeline:

1. **LIR arrives** from the middle-end with live range information.
2. **Flow network is constructed** from the LIR.
3. **MCF is solved** using SSP with potentials.
4. **Register assignments are extracted** from the flow.
5. **Spill code is inserted** into the LIR.
6. **LIR is updated** with physical register indices.
7. **Instruction scheduling** reorders instructions to hide latency (using the register assignments).
8. **Machine IR is emitted** with the final register assignments.

---

### 10. Rust Implementation Structure

```rust
// Core data structures
pub struct FlowNetwork { ... }
pub struct FlowNode { ... }
pub struct FlowEdge { ... }

// MCF solver
pub struct McfSolver { ... }
impl McfSolver {
    pub fn new(network: FlowNetwork) -> Self { ... }
    pub fn solve(&mut self) -> Result<Flow, McfError> { ... }
    fn dijkstra_with_potentials(&self, target: usize) -> (Vec<usize>, Vec<f64>) { ... }
    fn augment_flow(&mut self, path: &[usize]) { ... }
    fn update_potentials(&mut self, dist: &[f64]) { ... }
}

// Register allocation
pub struct RegisterAllocator { ... }
impl RegisterAllocator {
    pub fn allocate(lir: &LirFunction, target: &TargetIsa) -> RegisterAllocation { ... }
    fn build_network(lir: &LirFunction, target: &TargetIsa) -> FlowNetwork { ... }
    fn extract_assignments(flow: &Flow, network: &FlowNetwork) -> HashMap<ValueId, RegisterId> { ... }
    fn insert_spill_code(lir: &mut LirFunction, flow: &Flow, network: &FlowNetwork) { ... }
}
```

---

This is the complete, comprehensive design for Min-Cost Multi-Commodity Flow register allocation. It covers the mathematical formulation, network construction, algorithm, integration with LIR, heterogeneous register classes, concurrency, performance optimizations, and edge cases. No glossing over.


## Instruction Selection: Complete Technical Design

### 1. Problem Formulation: DAG Covering

Instruction selection is formulated as a **minimum-cost DAG covering problem**. Given a directed acyclic graph (DAG) representing the LIR computation, find a set of tree patterns (instruction templates) that cover all nodes in the DAG with minimum total cost.

**Given:**
- A DAG $G = (V, E)$ where each node $v \in V$ represents an LIR operation
- A set of tree patterns $P = \{p_1, p_2, \ldots, p_k\}$ where each pattern $p_i$ is a tree that can match a subgraph of $G$
- A cost function $cost: P \rightarrow \mathbb{R}^+$ representing the cost (latency, code size, or weighted combination) of each pattern
- A set of constraints $C$ (e.g., operand requirements, register class restrictions)

**Objective:**
Find a covering $S \subseteq P$ such that:
1. Every node in $G$ is covered by at least one pattern in $S$
2. The total cost $\sum_{p \in S} cost(p)$ is minimized
3. All constraints in $C$ are satisfied

**Key Insight:**
Unlike tree covering (which is solvable in polynomial time via DP), DAG covering is NP-hard in general. However, by restricting patterns to be trees and using a bottom-up DP approach with memoization, we can solve it efficiently for practical instruction sets.

---

### 2. The DP-on-DAG Algorithm

We use **dynamic programming on the DAG** with bottom-up traversal and memoization. The algorithm computes the minimum cost to produce the value at each node, considering all possible patterns that can cover that node and its descendants.

#### 2.1 Core Data Structures

**ValueCell:**
For each node $v \in V$, we maintain a `ValueCell` that stores:
- `form`: The winning pattern (e.g., `AddImm`, `Lea2`, `LoadIndexed`)
- `cost`: The minimum cost to produce this node's value
- `operands`: The operands consumed by this pattern (with their materialization state)
- `instrs`: Estimated instruction count (for tie-breaking)
- `rank`: Heuristic quality score (for final tie-breaking)

```rust
pub struct ValueCell {
    pub form: PatternForm,
    pub cost: i32,
    pub operands: Vec<OperandRef>,
    pub instrs: u32,
    pub rank: u32,
}
```

**OperandRef:**
Operands can be in different materialization states:
- `Chain`: The operand is computed inline as part of the parent instruction (zero additional cost)
- `Leaf`: The operand must be materialized to a register or loaded from memory
- `Imm`: The operand is a valid immediate for this instruction

```rust
pub enum OperandRef {
    Chain { node: NodeId },
    Leaf { node: NodeId },
    Imm { value: i64 },
}
```

**PatternForm:**
The specific pattern that won (e.g., `AddImm`, `Lea2`, `LoadIndexed`, `MagicDiv`).

```rust
pub enum PatternForm {
    Leaf,
    AddImm { base: OperandRef, imm: i64 },
    Lea2 { base: OperandRef, index: OperandRef, scale: u8, disp: i32 },
    MagicDiv { dividend: OperandRef, divisor: i64, magic: u64, shift: u8 },
    // ... other patterns
}
```

#### 2.2 Bottom-Up Traversal

The algorithm processes nodes in **reverse topological order** (post-order traversal of the DAG). For each node, it:
1. Identifies all patterns that can match this node
2. For each pattern, recursively computes the cost of the operands
3. Selects the pattern with minimum total cost
4. Memoizes the result in the `ValueCell`

```rust
pub fn select_instructions(dag: &Dag, target: &TargetIsa) -> Vec<ValueCell> {
    let mut cells = vec![None; dag.nodes.len()];
    let order = topological_sort_postorder(dag);
    
    for node_id in order {
        let node = &dag.nodes[node_id];
        let best = evaluate_all_patterns(node, dag, target, &cells);
        cells[node_id] = Some(best);
    }
    
    cells.into_iter().map(|c| c.unwrap()).collect()
}
```

#### 2.3 Pattern Evaluation

For each node, we evaluate all patterns from the target's rule set that can match this node's opcode and type.

```rust
fn evaluate_all_patterns(
    node: &DagNode,
    dag: &Dag,
    target: &TargetIsa,
    cells: &[Option<ValueCell>],
) -> ValueCell {
    let mut best = ValueCell {
        form: PatternForm::Leaf,
        cost: i32::MAX,
        operands: vec![],
        instrs: u32::MAX,
        rank: u32::MAX,
    };
    
    for pattern in target.patterns_for_opcode(node.opcode) {
        if !pattern.matches(node, dag) {
            continue;
        }
        
        let (cost, operands, instrs, rank) = evaluate_pattern(pattern, node, dag, cells);
        
        if cost < best.cost || 
           (cost == best.cost && instrs < best.instrs) ||
           (cost == best.cost && instrs == best.instrs && rank < best.rank) {
            best = ValueCell {
                form: pattern.form.clone(),
                cost,
                operands,
                instrs,
                rank,
            };
        }
    }
    
    best
}
```

#### 2.4 Operand Cost Computation

The cost of an operand depends on its materialization state:

**Chain Operand:**
The operand is computed inline as part of the parent instruction. Cost = 0 (the parent pattern's cost already includes it).

**Leaf Operand:**
The operand must be materialized. Cost depends on the operand's type:
- If it's a constant: cost = `target.cost_imm()` (load immediate)
- If it's a computed value: cost = `target.cost_load()` (load from register/spill slot)

**Imm Operand:**
The operand is a valid immediate for this instruction. Cost = 0 (no additional instruction needed).

```rust
fn operand_cost(op: &OperandRef, dag: &Dag, target: &TargetIsa) -> i32 {
    match op {
        OperandRef::Chain { node } => 0,
        OperandRef::Leaf { node } => {
            let operand_node = &dag.nodes[*node];
            if operand_node.is_constant() {
                target.cost_imm()
            } else {
                target.cost_load()
            }
        }
        OperandRef::Imm { .. } => 0,
    }
}
```

#### 2.5 Chain Folding

Chain folding is the key optimization that makes DP-on-DAG superior to simple tree covering. When a node has only one user, it can be "folded" into the parent instruction as a chain operand.

**Single-User Check:**
Before folding a node as a chain, we verify it has exactly one user (the parent node).

```rust
fn can_fold_as_chain(node_id: NodeId, parent_id: NodeId, dag: &Dag) -> bool {
    let users = dag.users_of(node_id);
    users.len() == 1 && users[0] == parent_id
}
```

**Chain Depth Limit:**
To prevent pathological cases (e.g., extremely deep chains that cause stack overflow), we limit the chain depth.

```rust
const MAX_CHAIN_DEPTH: usize = 96;

fn try_commit(
    node_id: NodeId,
    dag: &Dag,
    target: &TargetIsa,
    cells: &mut [Option<ValueCell>],
    depth: usize,
) -> bool {
    if depth > MAX_CHAIN_DEPTH {
        return false;
    }
    
    // ... evaluate patterns
}
```

#### 2.6 Pattern Matching

Each pattern in the target's rule set defines:
- **Opcode match:** Which LIR opcodes this pattern can match
- **Type match:** Which data types this pattern supports
- **Operand constraints:** Requirements on the operands (e.g., "operand B must be a constant", "operand A must be a register")
- **Cost:** The latency/code size cost of this pattern

**Example: x86 LEA Pattern**

```rust
pub struct Lea2Pattern {
    // Matches: Add(Mul(x, Const(2^k)), y) or Add(Add(x, y), Const(disp))
}

impl Pattern for Lea2Pattern {
    fn matches(&self, node: &DagNode, dag: &Dag) -> bool {
        if node.opcode != Opcode::Add {
            return false;
        }
        
        // Check if this is a SIB-style address: base + index*scale + disp
        let (base, index, scale, disp) = match_lea_structure(node, dag);
        
        base.is_some() && index.is_some() && scale.is_some()
    }
    
    fn evaluate(
        &self,
        node: &DagNode,
        dag: &Dag,
        cells: &[Option<ValueCell>],
    ) -> (i32, Vec<OperandRef>, u32, u32) {
        let (base, index, scale, disp) = match_lea_structure(node, dag).unwrap();
        
        let base_op = if can_fold_as_chain(base, node.id, dag) {
            OperandRef::Chain { node: base }
        } else {
            OperandRef::Leaf { node: base }
        };
        
        let index_op = if can_fold_as_chain(index, node.id, dag) {
            OperandRef::Chain { node: index }
        } else {
            OperandRef::Leaf { node: index }
        };
        
        let cost = operand_cost(&base_op, dag, target) + 
                   operand_cost(&index_op, dag, target) + 
                   target.cost_op();  // LEA instruction cost
        
        let operands = vec![base_op, index_op];
        let instrs = 1;  // One LEA instruction
        let rank = 3;  // High quality pattern
        
        (cost, operands, instrs, rank)
    }
}
```

---

### 3. Target Rule Set Definition

Each target (x86, ARM, CUDA, custom ISA) defines its own rule set. The rule set is a collection of patterns, each with matching logic and cost.

#### 3.1 Rule Set Structure

```rust
pub struct TargetRuleSet {
    pub patterns: Vec<Box<dyn Pattern>>,
    pub opcode_to_patterns: HashMap<Opcode, Vec<usize>>,  // opcode -> pattern indices
}

impl TargetRuleSet {
    pub fn patterns_for_opcode(&self, opcode: Opcode) -> Vec<&dyn Pattern> {
        self.opcode_to_patterns
            .get(&opcode)
            .map(|indices| indices.iter().map(|&i| self.patterns[i].as_ref()).collect())
            .unwrap_or_default()
    }
}
```

#### 3.2 Example: x86-64 Rule Set

```rust
pub fn x86_64_rule_set() -> TargetRuleSet {
    let patterns: Vec<Box<dyn Pattern>> = vec![
        Box::new(AddImmPattern),
        Box::new(Lea2Pattern),
        Box::new(LeaRRPattern),
        Box::new(ShlImmPattern),
        Box::new(MagicDivPattern),
        Box::new(GenericArithPattern),
        // ... more patterns
    ];
    
    let mut opcode_to_patterns = HashMap::new();
    
    for (i, pattern) in patterns.iter().enumerate() {
        for opcode in pattern.supported_opcodes() {
            opcode_to_patterns.entry(opcode).or_insert_with(Vec::new).push(i);
        }
    }
    
    TargetRuleSet { patterns, opcode_to_patterns }
}
```

#### 3.3 Cost Model

The cost model is target-specific and encodes the microarchitectural characteristics:

```rust
pub trait TargetCostModel {
    fn cost_load(&self) -> i32;
    fn cost_store(&self) -> i32;
    fn cost_imm(&self) -> i32;
    fn cost_op(&self) -> i32;
    fn cost_mul(&self) -> i32;
    fn cost_div(&self) -> i32;
    fn cost_move(&self) -> i32;
    
    fn is_valid_immediate(&self, opcode: Opcode, value: i64) -> bool;
    fn max_chain_depth(&self) -> usize;
}

pub struct X86CostModel;

impl TargetCostModel for X86CostModel {
    fn cost_load(&self) -> i32 { 1 }   // mov reg, [mem]
    fn cost_store(&self) -> i32 { 1 }  // mov [mem], reg
    fn cost_imm(&self) -> i32 { 1 }    // mov reg, imm
    fn cost_op(&self) -> i32 { 1 }     // add, sub, lea, etc.
    fn cost_mul(&self) -> i32 { 3 }    // imul (higher latency)
    fn cost_div(&self) -> i32 { 10 }   // idiv (very high latency)
    fn cost_move(&self) -> i32 { 1 }   // mov reg, reg
    
    fn is_valid_immediate(&self, opcode: Opcode, value: i64) -> bool {
        // x86 supports sign-extended 32-bit immediates for most ops
        value >= i32::MIN as i64 && value <= i32::MAX as i64
    }
    
    fn max_chain_depth(&self) -> usize { 96 }
}
```

---

### 4. Integration with LIR

The instruction selector operates on the LIR (Low-level Intermediate Representation) after it has been linearized into basic blocks.

#### 4.1 LIR Structure

```rust
pub struct LirFunction {
    pub blocks: Vec<LirBlock>,
    pub values: Vec<LirValue>,
}

pub struct LirBlock {
    pub id: BlockId,
    pub instructions: Vec<LirInstruction>,
    pub successors: Vec<BlockId>,
}

pub struct LirInstruction {
    pub id: NodeId,
    pub opcode: Opcode,
    pub operands: Vec<OperandId>,
    pub result: Option<ValueId>,
    pub ty: Type,
}

pub struct LirValue {
    pub id: ValueId,
    pub def: NodeId,
    pub uses: Vec<NodeId>,
    pub ty: Type,
    pub register_class: RegisterClass,
}
```

#### 4.2 DAG Construction

Before instruction selection, we construct a DAG for each basic block:

```rust
pub fn build_dag(block: &LirBlock, func: &LirFunction) -> Dag {
    let mut dag = Dag::new();
    
    for instr in &block.instructions {
        let node_id = dag.add_node(DagNode {
            id: instr.id,
            opcode: instr.opcode,
            ty: instr.ty,
            operands: instr.operands.clone(),
        });
        
        for &operand_id in &instr.operands {
            let operand_value = &func.values[operand_id];
            if operand_value.def != instr.id {
                dag.add_edge(operand_value.def, node_id);
            }
        }
    }
    
    dag
}
```

#### 4.3 Instruction Selection Pass

The instruction selection pass processes each basic block independently:

```rust
pub fn instruction_selection(lir: &mut LirFunction, target: &TargetIsa) {
    for block in &mut lir.blocks {
        let dag = build_dag(block, lir);
        let cells = select_instructions(&dag, target);
        
        // Annotate LIR instructions with selected patterns
        for (i, instr) in block.instructions.iter_mut().enumerate() {
            instr.selected_pattern = Some(cells[i].form.clone());
            instr.selected_operands = Some(cells[i].operands.clone());
        }
    }
}
```

---

### 5. Heterogeneous Target Support

The instruction selector is target-agnostic. It works for any target that provides a `TargetRuleSet` and `TargetCostModel`.

#### 5.1 x86-64 Specifics

- **SIB Addressing:** LEA patterns that match `base + index*scale + disp`
- **Magic Division:** Replace division by constant with multiply-by-magic-number
- **Shift Optimization:** Replace multiply by power-of-2 with shift
- **Immediate Forms:** Use `add reg, imm` instead of `mov reg, imm; add reg, reg`

#### 5.2 ARM64 Specifics

- **Shifted Operands:** `add x0, x1, x2, lsl #2` (add with shifted operand)
- **Extended Registers:** `add x0, x1, w2, sxtw` (add with sign-extended register)
- **Load/Store Pair:** `ldp x0, x1, [sp]` (load pair of registers)
- **Conditional Instructions:** `csel x0, x1, x2, eq` (conditional select)

#### 5.3 GPU Specifics (CUDA)

- **Fused Multiply-Add:** `fma.rn.f32 x0, x1, x2, x3` (fused multiply-add)
- **Warp-Level Primitives:** `shfl.sync.up.b32 x0, x1, offset, mask`
- **Shared Memory:** `ld.shared.f32 x0, [addr]` (shared memory load)
- **Tensor Core:** `mma.sync.aligned.m16n8k16.row.col.f32.f16.f16.f32`

#### 5.4 Custom ISA Specifics

For a custom accelerator, the rule set might include:
- **Systolic Array Operations:** `systolic_mac tile_a, tile_b, tile_c`
- **DMA Transfers:** `dma_load dst, src, size`
- **Specialized Instructions:** `fft_1d input, output, size`

---

### 6. Concurrency Model

Instruction selection is inherently parallel at the basic block level. Each block's DAG is independent, so we can process thousands of blocks concurrently.

#### 6.1 Function-Level Parallelism

The work-stealing scheduler distributes functions to worker threads:

```rust
pub fn select_instructions_parallel(lir: &mut LirFunction, target: &TargetIsa) {
    let blocks: Vec<_> = lir.blocks.iter_mut().collect();
    
    blocks.par_iter_mut().for_each(|block| {
        let dag = build_dag(block, lir);
        let cells = select_instructions(&dag, target);
        
        for (i, instr) in block.instructions.iter_mut().enumerate() {
            instr.selected_pattern = Some(cells[i].form.clone());
            instr.selected_operands = Some(cells[i].operands.clone());
        }
    });
}
```

#### 6.2 Thread-Local State

Each worker thread maintains its own:
- `ValueCell` array (indexed by NodeId)
- DAG construction scratch space
- Pattern matching state

This eliminates synchronization overhead.

---

### 7. Performance Optimizations

#### 7.1 Memoization

The `ValueCell` array is indexed directly by `NodeId`, making lookups O(1) with no hashing overhead.

```rust
let mut cells = vec![None; dag.nodes.len()];
```

#### 7.2 Early Termination

If a node's best cost is already 0 (e.g., it's a constant that can be embedded as an immediate), we skip further pattern evaluation.

```rust
if best.cost == 0 {
    break;
}
```

#### 7.3 Pattern Ordering

Patterns are ordered by likelihood of matching. Common patterns (e.g., `AddImm`) are checked first to reduce the number of pattern matches.

#### 7.4 Sparse DAG Representation

The DAG uses adjacency lists instead of adjacency matrices to save memory and improve cache locality.

```rust
pub struct Dag {
    pub nodes: Vec<DagNode>,
    pub edges: Vec<Vec<NodeId>>,  // node -> list of successor nodes
    pub reverse_edges: Vec<Vec<NodeId>>,  // node -> list of predecessor nodes
}
```

---

### 8. Edge Cases and Correctness

#### 8.1 Multi-Use Nodes

If a node has multiple users, it cannot be folded as a chain. It must be materialized as a leaf.

```rust
if dag.users_of(node_id).len() > 1 {
    // Cannot fold as chain
    return OperandRef::Leaf { node: node_id };
}
```

#### 8.2 Cycles in the DAG

The DAG should be acyclic by construction (it's derived from SSA form). If a cycle is detected, it indicates a bug in the LIR.

#### 8.3 Infeasible Patterns

If no pattern matches a node, the node falls back to a `Leaf` pattern (load from register/spill slot). This should never happen if the target rule set is complete.

#### 8.4 Tie-Breaking

When multiple patterns have the same cost, we use:
1. Instruction count (prefer fewer instructions)
2. Rank (heuristic quality score)
3. Pattern index (deterministic tie-breaking)

---

### 9. Complete Backend Integration

The instruction selector is one component of the backend. The full backend pipeline:

1. **LIR arrives** from the middle-end with basic blocks and instructions.
2. **DAG is constructed** for each basic block.
3. **Instruction selection** runs DP-on-DAG to select patterns for each node.
4. **LIR is annotated** with selected patterns and operands.
5. **Register allocation** assigns physical registers to values.
6. **Spill code is inserted** for values that don't fit in registers.
7. **Instruction scheduling** reorders instructions to hide latency.
8. **Machine IR is emitted** with the final instruction sequence.

---

### 10. Rust Implementation Structure

```rust
// Core data structures
pub struct Dag { ... }
pub struct DagNode { ... }
pub struct ValueCell { ... }
pub enum OperandRef { ... }
pub enum PatternForm { ... }

// Pattern matching
pub trait Pattern { ... }
pub struct AddImmPattern { ... }
pub struct Lea2Pattern { ... }
pub struct MagicDivPattern { ... }

// Target rule set
pub struct TargetRuleSet { ... }
pub trait TargetCostModel { ... }
pub struct X86CostModel { ... }
pub struct Arm64CostModel { ... }

// Instruction selection
pub fn select_instructions(dag: &Dag, target: &TargetIsa) -> Vec<ValueCell> { ... }
fn evaluate_all_patterns(...) -> ValueCell { ... }
fn evaluate_pattern(...) -> (i32, Vec<OperandRef>, u32, u32) { ... }
fn operand_cost(...) -> i32 { ... }
fn can_fold_as_chain(...) -> bool { ... }

// Integration
pub fn instruction_selection(lir: &mut LirFunction, target: &TargetIsa) { ... }
pub fn build_dag(block: &LirBlock, func: &LirFunction) -> Dag { ... }
```

---

This is the complete, comprehensive design for instruction selection using DP-on-DAG. It covers the mathematical formulation, algorithm details, data structures, integration with LIR, target-specific aspects, concurrency, performance optimizations, and edge cases. No glossing over.


Here is the complete, end-to-end architectural and mathematical design of the Autotuner, strictly adhering to the "no code, maximum performance, single ML model" constraints.

---

### 1. The Search Space: Mixed Continuous-Discrete Formulation

The autotuner searches a unified configuration space (the "Genome") defined as $X = [\theta, I, F]$.

*   **Pass Ordering ($\theta$):** Modeled via the **Plackett-Luce (PL) distribution**. Instead of searching $O(N!)$ discrete permutations, the system maintains a continuous "skill score" (logit) $\theta_i$ for each of the $N$ compiler passes. The probability of a specific ordering is the product of softmax probabilities of selecting the next pass from the remaining pool. This allows the system to use gradient ascent to smoothly optimize the ordering.
*   **Iterations ($I$):** A vector of bounded discrete integers (e.g., 1 to 5) representing how many times a specific pass is applied consecutively.
*   **Factors ($F$):** A vector of bounded continuous scalars (e.g., tile size from 1.0 to 1024.0, unroll factor from 1.0 to 64.0).

---

### 2. The Surrogate Model: Conditional Graph Transformer (CGT)

This is the **single ML model** in the entire system. It does not generate code or replace compiler passes. It acts purely as a high-speed filter and predictor.

*   **Input State:** The Heterogeneous IR graph. Nodes carry features (opcode, data type, estimated latency, register pressure). Edges carry features (dependency type, spatial locality stride, communication volume).
*   **Input Action:** A dense vector embedding of the proposed Genome (the specific pass, iteration count, and factor values).
*   **Mechanism (Cross-Attention):** The Action embedding acts as the Query. The IR graph nodes and edges act as the Keys and Values. This forces the model to dynamically focus its computational capacity only on the subgraph that the proposed pass will actually mutate (e.g., attending heavily to loop headers and memory nodes if the action is "Loop Tiling").
*   **Dual-Head Output:**
    1.  **Delta Head:** Predicts the topological change in the IR (e.g., "node count will decrease by 15%, register pressure will increase by 2"). This is a vector of changes across all heterogeneous node/edge types.
    2.  **Value Head:** Predicts the log-execution time and, crucially, the **epistemic uncertainty** (variance) of that prediction.

---

### 3. The Acquisition Function: Multi-Fidelity Predictive Entropy Search (MF-PES)

Standard Bayesian Optimization greedily exploits known good areas. MF-PES actively seeks to resolve the surrogate model's greatest ambiguities, preventing the autotuner from getting stuck in local minima.

*   **Mathematical Core:** It calculates the mutual information between the unknown true global optimum and the potential observation at a candidate configuration. It asks: "Which configuration will teach us the most about where the true best configuration lies?"
*   **Approximation:** Since the true optimum is unknown, the system uses Monte Carlo sampling via CGT dropout. It generates multiple plausible "worlds" (variations of the surrogate model), finds the hypothetical optimum in each world, and averages the expected entropy reduction.
*   **Mixed-Space Optimization:** To find the next configuration to evaluate, the system uses alternating coordinate optimization:
    *   Gradient ascent updates the continuous PL logits ($\theta$) and continuous factors ($F$).
    *   Local neighborhood search evaluates $I-1, I, I+1$ to optimize the discrete iterations.
*   **The Multi-Fidelity Twist:** The acquisition value is divided by the evaluation cost $c(s)$ of the fidelity tier $s$. Early in the search, uncertainty is high, so the information gain is high for all tiers. Because Tier 1 is microscopically cheap, the ratio heavily favors it. Late in the search, uncertainty drops, and the ratio shifts to favor expensive Tier 3 silicon runs to break ties between nearly identical top contenders.

---

### 4. The 4-Tier Asynchronous Evaluation Funnel

The funnel ruthlessly filters configurations to minimize compile time, using zero-overhead handoffs.

*   **Tier 0: Surrogate Inference (Nanoseconds).** The PL model proposes thousands of candidate Genomes. The CGT evaluates them in a single, batched matrix multiplication. The bottom 90% are discarded instantly based on low predicted value or high uncertainty.
*   **Tier 1: Partial IR Execution (Microseconds).** In-process, lock-free worker threads apply only the first 30% of the proposed pass sequence to a thread-local copy of the Heterogeneous IR. The system measures the *actual* topological delta. If the actual delta deviates from the CGT's Delta Head prediction beyond a strict threshold $\epsilon$, the configuration is immediately discarded as a surrogate hallucination. Zero context switches.
*   **Tier 2: Cycle-Accurate Simulation (Milliseconds).** Surviving configurations are handed off via shared memory to an isolated simulator process. The simulator runs the full pass sequence to completion and models the target microarchitecture (cache lines, branch predictors, pipeline stages). It returns simulated cycle counts and cache miss rates.
*   **Tier 3: Silicon Validation (Seconds).** The top contenders are compiled to machine code and executed on actual hardware. The execution environment is strictly isolated: the CPU core is isolated from the OS scheduler, interrupt affinity is redirected, frequency scaling is locked, and turbo boost is disabled. The benchmark harness reads Hardware Performance Counters (PMCs) via memory-mapped pages for zero-kernel-overhead ground truth.

---

### 5. Transfer Learning: Topological Warm-Starting

Starting the Plackett-Luce logits from random values wastes expensive Tier 3 evaluations. The system warm-starts the search using historical data.

*   **Heterogeneous Graph MinHashing:** When a new function arrives, the system computes a structural fingerprint of its Heterogeneous IR. This fingerprint captures both the global distribution of node/edge types and the local neighborhood topology (e.g., distinguishing a compute-heavy loop from a memory-bound loop, even if their control flow graphs look identical).
*   **Similarity Retrieval:** The system queries a historical database using Jaccard similarity to find the top-5 most semantically similar functions ever compiled.
*   **Prior Injection:** It extracts the winning $\theta$ scores (pass orderings) and factor values from those past runs. These are injected as the initial prior distribution for the PL model and the CGT. The search begins at approximately 80% of the optimal solution, requiring only minor fine-tuning.

---

### 6. Concurrency and Execution Engine

To achieve maximum performance, the autotuner operates as a highly pipelined, asynchronous controller.

*   **Asynchronous Pipelining:** The controller does not wait for sequential completion. While Tier 3 is validating Genome A on silicon, Tier 2 is simulating Genomes B through E, and Tier 0/1 are filtering Genomes F through Z.
*   **Lock-Free Data Flow:** 
    *   Tier 1 evaluations use thread-local bump allocators and copy-on-write IR semantics. 
    *   Tier 2 and Tier 3 handoffs use pre-allocated, memory-mapped shared memory segments with atomic flag synchronization (or `futex`), entirely bypassing OS IPC overhead.
*   **Zero-Allocation Hot Path:** The CGT inference and MF-PES calculations operate on pre-allocated, contiguous memory blocks. There is no dynamic memory allocation during the search loop, guaranteeing deterministic, jitter-free latency.
*   **Background Model Updating:** Ground-truth data from Tier 3 is appended to a ring buffer. A dedicated, low-priority background thread periodically retrains the CGT and updates the MinHash database, ensuring the model continuously adapts without blocking the main compilation pipeline.

---

### Summary of the Autotuner's Operational Loop

1. A function's Heterogeneous IR is hashed and matched against historical data to warm-start the PL logits.
2. The PL model samples a batch of candidate Genomes.
3. The CGT (Tier 0) instantly filters out the bottom 90%.
4. Worker threads execute Tier 1 partial evaluations, discarding configurations where the actual IR transformation mismatches the predicted delta.
5. The MF-PES acquisition function selects the most informative survivors for Tier 2 simulation.
6. The top Tier 2 survivors are promoted to Tier 3 for isolated, PMC-validated silicon execution.
7. The ground-truth execution times update the PL logits via gradient ascent and are logged to the historical database for future transfer learning.

This design guarantees that the compiler spends its time evaluating only the configurations mathematically most likely to yield the absolute best machine code, while strictly bounding compile time through aggressive, multi-fidelity pruning.


### The Borrow Checker & Lifetime Analysis: Complete Architectural Design

To achieve maximum performance without external libraries, we must discard the naive, rule-based approach of early compiler designs. Instead, we formulate the Rust Borrow Checker as a **Graph Constraint Satisfaction Problem (CSP)** operating directly on the Sea of Nodes. 

This design integrates seamlessly with your Heterogeneous IR, ensuring that memory safety is not an afterthought, but a mathematically proven property of the graph before it ever reaches the backend.

---

### 1. The Core Paradigm: Borrow Checking as Graph Constraint Satisfaction

The borrow checker is divided into two distinct, mathematically rigorous phases:
1.  **Region Inference:** Computing the exact, minimal bounds of every lifetime (Region) in the program.
2.  **Borrow Validation:** Proving that the aliasing rules (no mutable aliasing, no use-after-free) hold true across the computed regions.

#### The Mathematical Model
Let $P$ be the set of all program points (nodes in the Sea of Nodes).
A **Region** (Lifetime) $R$ is defined as a subset of $P$ ($R \subseteq P$).
A **Borrow** is an edge in the graph labeled with a Region $R$ and a Borrow Kind $K \in \{Shared, Mutable\}$.

The compiler generates a set of constraints during semantic analysis:
*   **Outlives Constraints ($R_1 \supseteq R_2$):** Region $R_1$ must contain all program points of $R_2$.
*   **Subset Constraints:** A value of type $T$ with region $R_1$ can be assigned to a location expecting type $T$ with region $R_2$ only if $R_1 \supseteq R_2$.
*   **Liveness Facts:** A region $R$ is "live" at program point $p$ if there is a path from a definition of $R$ to a use of $R$ that passes through $p$.

---

### 2. IR Representation: Lifetimes in the Sea of Nodes

We do not use a separate data structure for the borrow checker. The borrow graph is embedded directly into the **Heterogeneous Sea of Nodes**.

*   **Region Nodes:** Special meta-nodes in the graph that represent the creation of a lifetime (e.g., entering a scope, creating a reference). They carry a unique `RegionId`.
*   **Borrow Edges:** Data-flow edges are augmented with a `BorrowKind` (Shared/Mutable) and a `RegionId`. 
*   **Drop Nodes:** Explicit nodes in the graph representing the destruction of a value. They are connected to the Region nodes to enforce destruction ordering.

By embedding the borrow graph into the Sea of Nodes, the borrow checker operates on the exact same memory layout as the optimization passes. There is no translation overhead.

---

### 3. Phase 1: Region Inference (Computing Exact Lifetimes)

The goal is to find the smallest possible Region $R$ for every `RegionId` that satisfies all Outlives and Subset constraints. This is equivalent to finding the **Transitive Closure** of a constraint graph.

#### The Max-Perf Algorithm: SCC Condensation + Bitset Propagation
A naive fixed-point iteration over the Control Flow Graph (CFG) is $O(N^2)$ and too slow. We use a highly optimized graph algorithm.

1.  **Constraint Graph Construction:** We build a directed graph where nodes are `RegionId`s and edges are Outlives constraints ($R_1 \rightarrow R_2$ means $R_1 \supseteq R_2$).
2.  **Tarjan’s Strongly Connected Components (SCC):** We run Tarjan’s algorithm to condense the constraint graph into a DAG of SCCs. All regions within an SCC must be exactly identical. This reduces the graph size dramatically and eliminates infinite loops in cyclic constraints.
3.  **Reverse Topological Propagation:** We process the DAG of SCCs in reverse topological order. 
4.  **Bitset Liveness:** For each Region, its extent (the set of program points where it is live) is represented as a **dense, contiguous bitset**. 
5.  **Set Union Operations:** As we propagate constraints up the DAG, we compute the liveness of a parent region by performing a bitwise OR of the bitsets of its children. 
    *   *Max Perf Detail:* Bitsets are aligned to 256-bit boundaries and processed using SIMD instructions (AVX2/AVX-512) directly in the thread-local arena. This makes the set union operations run at memory-bandwidth speeds.

**Result:** Every `RegionId` is now annotated with an exact, minimal bitset of program points where it is live.

---

### 4. Phase 2: The Borrow Conflict Graph (Proving Safety)

Now that we know exactly where every lifetime is live, we must prove that the aliasing rules are never violated. This is formulated as an **Interference Graph** problem.

#### The Validation Rules
For every Mutable Borrow edge $E_m$ with Region $R_m$:
1.  **No Mutable Alias:** There must be no other Mutable Borrow edge $E_{m2}$ with Region $R_{m2}$ such that $R_m \cap R_{m2} \neq \emptyset$.
2.  **No Shared Alias:** There must be no Shared Borrow edge $E_s$ with Region $R_s$ such that $R_m \cap R_s \neq \emptyset$.
3.  **No Use After Move:** The underlying value must not be accessed outside of $R_m$.

#### The Max-Perf Algorithm: Liveness Intersection
1.  **Borrow Node Extraction:** We extract all Borrow and Access nodes from the Sea of Nodes.
2.  **Interference Matrix:** We build a sparse interference matrix. For each Mutable Borrow, we check its liveness bitset against the liveness bitsets of all other Borrows to the same underlying memory location.
3.  **Bitwise AND for Conflicts:** To check for overlap between Region $A$ and Region $B$, we perform a bitwise AND on their bitsets. If the result is non-zero, a conflict exists.
4.  **Early Exit & Error Reporting:** If a conflict is found, the checker immediately halts for that function and generates a diagnostic. The diagnostic engine traces the conflicting bitsets back to the exact source spans in the AST.

**Handling Rust-Specific Edge Cases:**
*   **Closures:** Captured variables are modeled as implicit struct arguments. The closure body is analyzed as a separate function, with the captured variables carrying their parent's Region constraints.
*   **Traits & Generics:** Trait bounds generate abstract Region constraints. During Monomorphization (which happens before the final borrow check), these abstract regions are unified with concrete regions, and the check is run on the monomorphized Sea of Nodes.
*   **Unsafe Blocks:** The borrow checker treats `unsafe` blocks as "trust boundaries." It verifies that the *interface* of the unsafe function is safe, but suspends aliasing checks *inside* the block, relying on the programmer's guarantee.

---

### 5. Phase 3: Drop Check and Destruction Ordering

Rust requires that values are destroyed in the exact reverse order of their creation, and that no value is destroyed while it is still borrowed.

1.  **Drop Graph Construction:** We build a directed graph where an edge from $A \rightarrow B$ means "$A$ must be dropped before $B$".
2.  **Cycle Detection:** We run a linear-time cycle detection algorithm (DFS with coloring) on the Drop Graph. If a cycle is found, it means there is a circular dependency in destruction (e.g., a struct containing a reference to itself), which is a compile-time error.
3.  **Drop Node Insertion:** If the Drop Graph is acyclic, we insert explicit `Drop` nodes into the Sea of Nodes at the exact program points where the Region bitsets end.

---

### 6. Max-Perf Execution Model (Concurrency & Data Structures)

To make the borrow checker run at maximum speed without external libraries, we leverage the custom concurrency engine designed for the rest of the compiler.

#### Function-Level Parallelism
Rust’s borrow checker is fundamentally **intra-procedural** (it analyzes one function at a time, using signatures for cross-function calls). This maps perfectly to our work-stealing scheduler.
*   The compiler distributes functions to worker threads.
*   Each thread runs the Region Inference and Borrow Validation entirely independently.
*   There is **zero synchronization** between threads during the borrow check.

#### Zero-Allocation Thread-Local Arenas
*   **Constraint Graphs & Bitsets:** Every worker thread has a pre-allocated, massive bump-allocator arena. The constraint graph, SCC structures, and liveness bitsets are allocated here.
*   **No Heap Fragmentation:** When the borrow check for a function completes, the thread simply resets the arena pointer to zero. The memory is instantly reclaimed with zero overhead, zero garbage collection, and zero fragmentation.

#### Cache Locality
*   The Sea of Nodes is stored in flat, contiguous arrays (indexed by `NodeId`). 
*   The borrow checker traverses these arrays sequentially. Because the bitsets and constraint graphs are allocated in the same thread-local arena, the entire working set of the borrow checker for a single function fits perfectly into the L1/L2 cache.

---

### 7. Integration with the Autotuner and Backend

The borrow checker is not an isolated phase; it directly informs the rest of the compiler.

*   **Informing the Backend (Register Allocation):** The exact liveness bitsets computed by the Region Inference phase are directly exported to the **Min-Cost Multi-Commodity Flow (MCF) Register Allocator**. The MCF solver uses these exact lifetimes to construct its flow network, eliminating the need for the backend to re-compute liveness.
*   **Informing the Autotuner:** The Autotuner’s ML Pruner uses the "Borrow Conflict Density" (how many overlapping lifetimes exist in a hot loop) as a feature. If the borrow checker reports high conflict density, the Autotuner knows to prioritize passes that reduce memory pressure or restructure data layouts to minimize aliasing.
*   **Semantic IR Lowering:** The borrow checker runs *before* the Heterogeneous IR is lowered to the target-specific LIR. It guarantees that the LIR and Machine IR are mathematically proven to be memory-safe, allowing the backend to aggressively optimize (e.g., removing bounds checks, eliminating atomics) without fear of introducing undefined behavior.

### Summary of the Borrow Checker Design

By formulating the borrow checker as a **Graph Constraint Satisfaction Problem** solved via **SCC Condensation** and **SIMD-accelerated Bitset Propagation**, we achieve a system that is:
1.  **Mathematically Rigorous:** It handles the full complexity of Rust's Non-Lexical Lifetimes, closures, and traits.
2.  **Max-Perf:** It runs in near-linear time, fully parallelized across functions, with zero heap allocations in the hot path.
3.  **Deeply Integrated:** It shares the exact same memory layout (Sea of Nodes) as the optimizer and backend, providing them with free, precise liveness data.

This completes the semantic core of the compiler. The source code is now parsed, semantically verified for memory safety, and represented as a mathematically sound, heterogeneous graph, ready to be fed into the Autotuner and the DP-on-DAG backend.


## Object File Emission & Linker: Complete Technical Design

### 1. The Object File Format Abstraction Layer

The compiler must emit object files in multiple formats (ELF for Linux, Mach-O for macOS, PE/COFF for Windows). Rather than hardcoding format-specific logic throughout the compiler, we define a **Format-Agnostic Object Model** that is serialized to the target format at the final emission stage.

#### The Universal Object Model

The object model consists of:
- **Sections:** Named, typed regions of the binary (e.g., `.text` for code, `.data` for initialized data, `.rodata` for constants, `.bss` for uninitialized data)
- **Symbols:** Named entities (functions, global variables, constants) with attributes (visibility, binding, section association, offset within section)
- **Relocations:** Patches that must be applied to the binary when final addresses are known (e.g., "patch this 32-bit offset to point to function X")
- **Debug Information:** Metadata mapping binary addresses back to source code (line numbers, variable locations, type information)

#### Format-Specific Serializers

Each target format (ELF, Mach-O, PE) implements a **Serializer** that converts the Universal Object Model into the format's binary layout:
- **ELF Serializer:** Writes ELF header, section headers, program headers, symbol table, string tables, relocation tables
- **Mach-O Serializer:** Writes Mach-O header, load commands, segment commands, section commands, symbol table, indirect symbol table
- **PE Serializer:** Writes PE header, COFF header, section table, export table, import table, debug directory

The serializers are deterministic and produce bit-for-bit identical output given the same input, which is critical for reproducible builds.

---

### 2. Section Layout & Alignment Algorithm

The compiler must decide how to arrange sections within the object file. This affects load time, cache locality, and page alignment.

#### The Layout Algorithm

1. **Section Classification:** Sections are classified by type:
   - **Executable (AX):** `.text` (code)
   - **Read-Only (A):** `.rodata`, `.eh_frame` (exception handling)
   - **Read-Write (WA):** `.data`, `.bss`
   - **Debug (non-allocatable):** `.debug_info`, `.debug_line`

2. **Alignment Constraints:** Each section has an alignment requirement (e.g., `.text` must be 16-byte aligned for instruction cache line boundaries, `.data` must be page-aligned for memory protection).

3. **Sorting Strategy:** Sections are sorted by:
   - **Allocatability:** Non-allocatable sections (debug info) are placed at the end
   - **Permissions:** Sections with different permissions (RX, RW, R) must be in separate segments
   - **Alignment:** Higher alignment requirements come first to minimize padding
   - **Hot/Cold Splitting:** Hot code (frequently executed) is placed at the beginning of `.text` for better instruction cache locality; cold code (error handlers, rarely-called functions) is placed at the end

4. **Padding Insertion:** The algorithm inserts zero-filled padding between sections to satisfy alignment requirements. The padding is minimized by ordering sections to reduce wasted space.

#### Max-Perf Layout

The layout algorithm runs in $O(N \log N)$ time (sorting) and uses a greedy approach to minimize padding. It operates on a thread-local arena and produces a deterministic layout regardless of input order.

---

### 3. Symbol Table & String Table Generation

Symbols and their names must be efficiently stored and indexed.

#### Symbol Table Structure

The symbol table is a flat, sorted array of symbol records. Each record contains:
- **Name Offset:** Index into the string table
- **Value:** Address or offset (resolved during linking)
- **Size:** Size of the symbol (for functions, the code size; for variables, the data size)
- **Binding:** Local (file-scoped) or Global (visible to other object files)
- **Type:** Function, Object (variable), Section, File
- **Visibility:** Default, Hidden, Protected
- **Section Index:** Which section contains this symbol

#### String Table Structure

The string table is a single, contiguous buffer of null-terminated strings. To minimize size:
- **String Deduplication:** Before inserting a string, the system checks if it already exists in the table (using a hash map). If it does, the existing offset is reused.
- **Suffix Sharing:** If string A is "foo" and string B is "foobar", the table stores "foobar" and points A to the suffix "foo" within it.

#### Max-Perf Generation

The symbol table is built concurrently:
1. Each worker thread (processing a function or global variable) emits symbols to a thread-local buffer
2. After all threads complete, the buffers are merged and sorted by name
3. The string table is built by concatenating all symbol names with deduplication
4. The final symbol table offsets are computed in a single pass

The entire process is $O(N \log N)$ for sorting and $O(N)$ for string deduplication, with zero heap allocations in the hot path.

---

### 4. Relocation System

Relocations are the mechanism by which the linker resolves references to external symbols or adjusts addresses when the final layout is known.

#### Relocation Types

Each target architecture defines its own relocation types. For x86-64:
- **R_X86_64_64:** 64-bit absolute address
- **R_X86_64_PC32:** 32-bit PC-relative offset
- **R_X86_64_GOTPCREL:** 32-bit offset to Global Offset Table entry
- **R_X86_64_PLT32:** 32-bit offset to Procedure Linkage Table entry

Each relocation record contains:
- **Offset:** Byte offset within the section where the patch must be applied
- **Symbol Index:** Index into the symbol table (which symbol's address to use)
- **Type:** The relocation type (how to compute the patch value)
- **Addend:** A constant to add to the computed value

#### Relocation Generation

During instruction selection and register allocation, the backend emits relocations for:
- **Function Calls:** When calling a function in another object file, emit a PC-relative relocation
- **Global Variable Access:** When accessing a global variable, emit an absolute or PC-relative relocation
- **Jump Tables:** For switch statements, emit relocations for each jump target

#### Relocation Resolution (Linker Phase)

During linking, the linker processes all relocations:
1. For each relocation, look up the symbol's final address
2. Compute the patch value based on the relocation type (e.g., for PC-relative: `symbol_address - (relocation_offset + 4)`)
3. Write the patch value to the specified offset in the section

This is a simple, deterministic $O(R)$ process where $R$ is the number of relocations.

---

### 5. Debug Information Emission (DWARF)

Debug information allows debuggers (gdb, lldb) to map binary addresses back to source code. The standard format is **DWARF** (Debugging With Attributed Record Formats).

#### DWARF Structure

DWARF consists of multiple sections:
- **.debug_info:** The core debug information (compilation units, subprograms, variables, types)
- **.debug_abbrev:** Abbreviation tables that compress the debug info
- **.debug_line:** Line number programs mapping addresses to source lines
- **.debug_str:** String table for debug info
- **.debug_ranges:** Non-contiguous address ranges for optimized code
- **.debug_loc:** Variable location descriptions (register, stack offset, etc.)

#### DWARF Generation Algorithm

1. **Compilation Unit (CU) Emission:** For each source file, emit a CU containing:
   - **DW_TAG_compile_unit:** The root of the debug tree, with attributes for the source file name, compilation directory, and producer (compiler name/version)
   - **DW_TAG_subprogram:** For each function, with attributes for the function name, source line, and address range
   - **DW_TAG_variable:** For each local/global variable, with attributes for the name, type, and location

2. **Type Information:** Emit DWARF type descriptions for all types used in the program:
   - **DW_TAG_base_type:** Primitive types (int, float, etc.)
   - **DW_TAG_structure_type:** Structs with member offsets
   - **DW_TAG_pointer_type:** Pointers with target type
   - **DW_TAG_typedef:** Type aliases

3. **Line Number Program:** Generate a state machine program that maps instruction addresses to source lines. The program uses opcodes like:
   - **DW_LNS_advance_pc:** Increment the address
   - **DW_LNS_advance_line:** Increment the line number
   - **DW_LNE_end_sequence:** Mark the end of a sequence

4. **Variable Location Lists:** For variables that move between registers and stack slots during execution (due to register allocation), emit location lists that describe where the variable is at each address range.

#### Max-Perf DWARF Generation

DWARF generation is notoriously slow in production compilers. Our approach:
- **Lazy Emission:** Debug info is only generated for symbols that are actually emitted (dead code elimination removes unused debug info)
- **Concurrent Generation:** Each worker thread generates debug info for its assigned functions in parallel
- **Compression:** The debug sections are compressed using zlib or zstd to reduce binary size
- **Abbreviation Optimization:** The abbreviation table is optimized to minimize the size of the debug info

---

### 6. The Linker: Symbol Resolution

The linker's first task is to resolve all symbol references across object files.

#### Symbol Resolution Algorithm

1. **Symbol Collection:** Collect all global symbols from all input object files into a global symbol table
2. **Conflict Detection:** If two object files define the same global symbol:
   - If both are "strong" definitions (not weak), emit a "multiple definition" error
   - If one is "weak" (e.g., inline functions, template instantiations), keep the strong definition
   - If both are weak, keep the first one encountered (deterministic based on input order)
3. **Undefined Symbol Resolution:** For each undefined symbol reference, look up the symbol in the global table. If not found, emit an "undefined reference" error
4. **Common Symbol Handling:** For "common" symbols (uninitialized global variables in C), allocate space in `.bss` and resolve all references to that single allocation

#### Max-Perf Resolution

The symbol resolution uses a concurrent hash map (sharded by symbol hash) to allow parallel symbol insertion and lookup. The conflict detection runs in $O(N)$ time where $N$ is the total number of symbols across all object files.

---

### 7. The Linker: Dead Code Elimination

The linker removes code and data that are never referenced, reducing binary size.

#### Garbage Collection Algorithm

1. **Root Identification:** Mark all symbols that are "roots" (e.g., `main`, exported symbols, constructors)
2. **Reachability Analysis:** Starting from the roots, traverse all relocations to find which sections are reachable. A section is reachable if:
   - It contains a root symbol, OR
   - It is referenced by a relocation from a reachable section
3. **Unreachable Section Removal:** Remove all sections that are not marked as reachable
4. **Symbol Table Cleanup:** Remove symbols that belong to removed sections

#### Section-Level Granularity

Dead code elimination operates at the **section level**, not the function level. To achieve function-level granularity, the compiler must emit each function in its own section (e.g., `.text.function_name`). This is controlled by the `-ffunction-sections` flag.

#### Max-Perf GC

The reachability analysis uses a worklist algorithm:
1. Initialize the worklist with all root sections
2. While the worklist is not empty:
   - Pop a section from the worklist
   - Mark it as reachable
   - For each relocation in the section, if the target section is not marked, add it to the worklist
3. This runs in $O(S + R)$ time where $S$ is the number of sections and $R$ is the number of relocations

---

### 8. The Linker: Address Assignment & Final Layout

The linker assigns final addresses to all sections and symbols.

#### Address Assignment Algorithm

1. **Segment Creation:** Group sections into segments based on permissions:
   - **Text Segment (RX):** `.text`, `.rodata`, `.eh_frame`
   - **Data Segment (RW):** `.data`, `.bss`
2. **Base Address Assignment:** Assign the base address of each segment (e.g., text segment at 0x400000 for x86-64 Linux)
3. **Section Layout:** Within each segment, lay out sections according to the alignment algorithm (Section 2)
4. **Symbol Address Computation:** For each symbol, compute its final address as `segment_base + section_offset + symbol_offset`
5. **Relocation Application:** Apply all relocations using the final symbol addresses (Section 4)

#### Position-Independent Code (PIC)

For shared libraries, the linker generates position-independent code:
- **Global Offset Table (GOT):** A table of addresses for global variables. Code references variables via the GOT
- **Procedure Linkage Table (PLT):** A table of stubs for calling external functions. The first call resolves the function address via the dynamic linker
- **Relative Relocations:** Use PC-relative relocations instead of absolute addresses

#### Max-Perf Layout

The address assignment is a single-pass algorithm that runs in $O(S + R)$ time. The relocation application is fully parallelized: each worker thread applies relocations for a subset of sections.

---

### 9. Link-Time Optimization (LTO) Integration

LTO allows the compiler to optimize across object file boundaries. Our LTO integrates with the autotuner.

#### LTO Process

1. **Bitcode Emission:** Instead of emitting Machine IR, the compiler emits a serialized representation of the Heterogeneous IR (the "bitcode")
2. **Bitcode Linking:** The linker reads all bitcode files and merges them into a single, whole-program Heterogeneous IR
3. **Whole-Program Optimization:** The autotuner runs on the merged IR, optimizing across function boundaries:
   - **Inlining:** Inline small functions across object file boundaries
   - **Devirtualization:** Resolve virtual function calls to direct calls
   - **Interprocedural Constant Propagation:** Propagate constants across functions
   - **Dead Code Elimination:** Remove unused functions and global variables
4. **Final Code Generation:** The optimized IR is lowered to Machine IR and emitted as a single object file

#### Max-Perf LTO

LTO is inherently sequential (it must merge all bitcode before optimizing). To maximize performance:
- **Parallel Bitcode Reading:** Multiple threads read and deserialize bitcode files concurrently
- **Incremental LTO:** Cache the merged IR and only re-optimize the parts that changed
- **ThinLTO:** Instead of merging all bitcode, perform a "thin" LTO that only inlines small functions and propagates constants, avoiding the full merge

---

### 10. Concurrency & Performance Model

The object file emission and linking are highly parallelizable.

#### Emission Concurrency

- **Per-Function Emission:** Each worker thread emits the Machine IR for its assigned functions directly into a thread-local buffer
- **Section Merging:** After all threads complete, the thread-local buffers are merged into the final sections. This is a simple concatenation with offset adjustment
- **Symbol Table Generation:** Symbols are collected concurrently and sorted in a single pass
- **Relocation Generation:** Relocations are generated concurrently with the code emission

#### Linking Concurrency

- **Parallel Symbol Resolution:** The global symbol table is built using a concurrent hash map
- **Parallel Dead Code Elimination:** The reachability analysis uses a concurrent worklist
- **Parallel Relocation Application:** Each worker thread applies relocations for a subset of sections
- **Parallel Debug Info Generation:** Debug info is generated concurrently for each compilation unit

#### Memory Model

- **Zero-Allocation Hot Path:** All emission and linking operations use pre-allocated, thread-local bump allocators
- **Memory-Mapped I/O:** The output file is memory-mapped, allowing direct writes without buffering
- **Cache-Friendly Layout:** The object model is stored in flat, contiguous arrays to maximize cache locality

---

### 11. Complete Integration Flow

The object file emission and linker integrate with the rest of the compiler as follows:

1. **Backend Output:** The instruction scheduler produces a sequence of Machine IR instructions with physical registers
2. **Binary Encoding:** The Machine Emitter encodes each instruction into binary (e.g., x86 machine code)
3. **Section Assignment:** Each encoded instruction is assigned to a section (e.g., `.text` for code, `.data` for initialized data)
4. **Symbol Emission:** Symbols are emitted for each function and global variable
5. **Relocation Emission:** Relocations are emitted for external references
6. **Debug Info Emission:** DWARF debug information is generated
7. **Object File Serialization:** The Universal Object Model is serialized to the target format (ELF/Mach-O/PE)
8. **Linking:** If multiple object files are being linked, the linker resolves symbols, eliminates dead code, assigns addresses, and applies relocations
9. **Final Binary:** The linked binary is written to disk

This completes the "chassis" of the compiler. The source code is now parsed, semantically verified, optimized, and emitted as a fully linked, debuggable executable.


## Partial Escape Analysis (PEA): Complete Technical Design

### 1. What is Partial Escape Analysis?

**Partial Escape Analysis** is a program transformation technique that determines, for each object allocation, on which **control-flow paths** the object escapes (becomes observable outside the current scope). Unlike traditional escape analysis which gives a binary answer ("escapes" or "doesn't escape"), PEA provides **path-sensitive** information:

- On path A: the object escapes → must be heap-allocated
- On path B: the object doesn't escape → can be scalar-replaced (allocated on stack or eliminated)

This is critical because many real-world programs have allocations that escape on error paths but not on the hot path. Traditional escape analysis would conservatively heap-allocate everything; PEA allows us to optimize the hot path while preserving correctness on cold paths.

**Mathematical Model:**
For each allocation site $a$ in the program, PEA computes a set of escape points $E(a) \subseteq P$ where $P$ is the set of program points. An allocation escapes at point $p$ if:
- The object is returned from the function
- The object is stored to a global variable
- The object is passed to a function that may retain it
- The object is thrown as an exception

The key insight: if $E(a) \cap \text{hot\_path} = \emptyset$, we can scalar-replace the allocation on the hot path, even if $E(a) \neq \emptyset$.

---

### 2. PEA in the Macro Expansion & Name Resolution Engine

The macro expansion and name resolution phases are allocation-heavy. They create:
- Temporary AST nodes during expansion
- Token streams for macro input/output
- Hygiene contexts for identifier scoping
- Symbol table entries during name resolution
- Temporary scope stacks during traversal

#### 2.1 The Problem: Allocation Overhead

Without PEA, every temporary structure is heap-allocated:
```
Macro expansion creates:
- TokenStream (heap)
- AST nodes (heap)
- Hygiene context (heap)
- Scope stack (heap)
```

Even though most of these temporaries are used only within the expansion phase and never escape, traditional analysis would heap-allocate them all.

#### 2.2 PEA Integration: Escape Edges in the Heterogeneous IR

We extend the Heterogeneous IR with **Escape Edges** that track object lifetime:

**New Edge Type: Escape Edge**
- Connects an allocation node to potential escape points
- Labeled with the escape kind (Return, GlobalStore, FunctionCall, Exception)
- Carries path information (which control-flow paths lead to this escape)

**New Node Type: Allocation Node**
- Represents heap allocation (e.g., `Box::new`, `Vec::new`)
- Annotated with the allocated type and size
- Connected to escape edges

#### 2.3 The PEA Algorithm for Macro Expansion

**Phase 1: Escape Point Computation**
For each allocation node $a$:
1. Perform a forward dataflow analysis from $a$
2. Track all paths where the allocated object is:
   - Returned from the function
   - Stored to a global/static variable
   - Passed to an unknown function (function pointer, trait object)
   - Stored to a field of an escaping object (transitive escape)
3. Record the escape points in a bitset (same SIMD-accelerated bitset infrastructure as the borrow checker)

**Phase 2: Path-Sensitive Scalar Replacement**
For each allocation node $a$ with escape set $E(a)$:
1. Compute the intersection $E(a) \cap \text{hot\_path}$
2. If the intersection is empty:
   - The allocation never escapes on the hot path
   - Replace the allocation with stack allocation or scalar replacement
   - Insert "materialization" nodes on cold paths where the object does escape
3. If the intersection is non-empty:
   - The allocation must remain heap-allocated
   - No transformation possible

**Phase 3: Materialization on Cold Paths**
When an allocation is scalar-replaced on the hot path but escapes on a cold path:
1. At the point where the cold path diverges from the hot path, insert a **Materialization Node**
2. The materialization node:
   - Allocates the object on the heap
   - Copies the scalar-replaced fields into the heap object
   - Returns a pointer to the heap object
3. This ensures correctness on the cold path while optimizing the hot path

#### 2.4 Application to Macro Expansion

**Token Streams:**
- Most token streams are consumed within the expansion phase
- PEA determines that token streams don't escape the expansion function
- Scalar-replace token streams: store tokens in a stack-allocated array instead of a heap-allocated `Vec`

**AST Nodes:**
- Temporary AST nodes created during expansion are often discarded
- PEA tracks which nodes are returned vs. which are consumed
- Non-escaping nodes are allocated in the thread-local arena (zero-cost)

**Hygiene Contexts:**
- Hygiene contexts are created for each macro invocation
- PEA determines that most contexts don't escape the expansion
- Scalar-replace contexts: store hygiene info in stack-allocated structs

**Result:** The macro expansion phase goes from $O(N)$ heap allocations to $O(1)$ heap allocations (only for objects that truly escape).

#### 2.5 Application to Name Resolution

**Symbol Table Entries:**
- During name resolution, we create temporary symbol table entries
- PEA determines which entries are permanent (exported symbols) vs. temporary (local variables)
- Temporary entries are allocated in the thread-local arena

**Scope Stacks:**
- Scope stacks are pushed/popped during traversal
- PEA determines that scope stacks don't escape the resolution function
- Scalar-replace scope stacks: use a fixed-size stack-allocated array with a pointer

**Result:** Name resolution becomes allocation-free on the hot path.

---

### 3. AOT Using PEA for the Runtime

The "runtime" refers to the compiler's own execution engine: the work-stealing scheduler, the IR manipulation passes, the autotuner, the backend.

#### 3.1 The Problem: Runtime Allocation Overhead

The runtime is allocation-heavy:
- IR nodes are allocated during pass execution
- Analysis results (dominator trees, liveness bitsets) are allocated
- Autotuner evaluations create temporary IR copies
- Backend passes allocate temporary data structures

Without PEA, all these allocations hit the heap, causing:
- Cache pollution
- Allocation overhead (even with bump allocators, there's overhead)
- GC pressure (if using a GC language)

#### 3.2 AOT Compilation with PEA

When we AOT-compile the runtime itself (the compiler binary), we apply PEA to the runtime code:

**Step 1: Static Analysis of Runtime Code**
- Parse the runtime source code (written in Rust)
- Build the Heterogeneous IR for the runtime
- Run PEA on the runtime IR

**Step 2: Identify Non-Escaping Allocations**
For each allocation in the runtime:
- Determine if it escapes (returned, stored globally, passed to unknown function)
- Compute the escape set $E(a)$

**Step 3: Transform the Runtime**
For allocations where $E(a) = \emptyset$:
- Replace heap allocation with stack allocation or scalar replacement
- Example: `let v = Vec::new()` becomes `let v: [T; N]` (stack-allocated array)

For allocations where $E(a) \cap \text{hot\_path} = \emptyset$:
- Scalar-replace on the hot path
- Insert materialization on cold paths

**Step 4: Emit Optimized Runtime**
- The transformed runtime is compiled to machine code
- The resulting binary has minimal heap allocations in the hot paths

#### 3.3 Specific Runtime Optimizations via PEA

**Work-Stealing Deques:**
- Chase-Lev deques allocate arrays for the queue
- PEA determines that deque arrays don't escape the worker thread
- Scalar-replace deque arrays: allocate in the thread-local arena

**IR Passes:**
- Passes create temporary IR nodes during transformation
- PEA determines which nodes are consumed vs. which are returned
- Non-escaping nodes are allocated in the thread-local arena

**Autotuner Evaluations:**
- Tier 1 evaluations create temporary IR copies
- PEA determines that these copies don't escape the evaluation function
- Scalar-replace IR copies: use stack-allocated buffers

**Backend Passes:**
- Instruction selection creates temporary `ValueCell` arrays
- PEA determines that these arrays don't escape the instruction selection function
- Scalar-replace `ValueCell` arrays: allocate on the stack

**Result:** The runtime becomes nearly allocation-free. The only remaining allocations are for objects that truly escape (e.g., the final Machine IR that is returned to the caller).

#### 3.4 Self-Hosting Implications

When the compiler compiles itself (self-hosting), it applies PEA to its own source code. This creates a **bootstrap cycle**:
1. Stage 0 (hand-written C++ compiler) compiles Stage 1 (Rust compiler)
2. Stage 1 applies PEA to its own source code during compilation
3. Stage 1 emits an optimized binary with minimal allocations
4. Stage 1 compiles itself again (Stage 2)
5. Stage 2 should produce a bit-for-bit identical binary (reproducible build)

The PEA transformations must be **deterministic** to ensure reproducible builds. This means:
- The escape analysis must produce the same results regardless of thread scheduling
- The scalar replacement decisions must be deterministic
- The materialization insertion must be deterministic

---

### 4. AOT Using PEA for Kernels

"Kernels" refers to GPU compute kernels or other accelerator code. These are particularly sensitive to allocations because:
- GPUs typically have no dynamic memory allocation (or very limited)
- Kernel performance is dominated by memory access patterns
- Register pressure is critical (GPUs have limited registers per thread)

#### 4.1 The Problem: Kernel Allocation Overhead

Kernel code often contains allocations:
```rust
// Example kernel
for i in 0..N {
    let temp = Vec::new();  // Allocation!
    temp.push(data[i]);
    result[i] = temp[0] * 2;
}
```

Without PEA, this allocation would:
- Fail on GPU (no heap)
- Or be very slow (if emulated via global memory)

#### 4.2 AOT Compilation of Kernels with PEA

When we AOT-compile kernel code, we apply PEA to eliminate allocations:

**Step 1: Kernel IR Construction**
- Parse the kernel source code
- Build the Heterogeneous IR with GPU-specific nodes (e.g., `WarpSync`, `SharedMemoryLoad`)

**Step 2: PEA on Kernel IR**
- Run PEA on the kernel IR
- Determine which allocations escape

**Step 3: Kernel-Specific Transformations**
For allocations where $E(a) = \emptyset$:
- **Scalar Replacement:** Replace the allocation with scalar variables
- **Shared Memory Promotion:** If the allocation is shared across threads in a block, promote it to shared memory
- **Register Promotion:** If the allocation is thread-local and small, promote it to registers

For allocations where $E(a) \neq \emptyset$:
- **Global Memory Allocation:** Allocate in global memory (slow, but correct)
- **Error Reporting:** Emit a compile-time error if the allocation is in a performance-critical path

**Step 4: Emit Optimized Kernel**
- The transformed kernel is compiled to GPU machine code (e.g., PTX for NVIDIA)
- The resulting kernel has minimal or zero allocations

#### 4.3 Specific Kernel Optimizations via PEA

**Temporary Arrays:**
- Many kernels create temporary arrays for intermediate results
- PEA determines that these arrays don't escape the kernel
- Scalar-replace small arrays: use registers
- Promote large arrays to shared memory

**Iterator State:**
- Rust iterators create state objects (e.g., `Map`, `Filter`)
- PEA determines that iterator state doesn't escape the loop
- Scalar-replace iterator state: inline the state into the loop

**Closure Captures:**
- Closures capture variables from the environment
- PEA determines which captures escape
- Non-escaping captures are scalar-replaced

**Result:** Kernels become allocation-free, with all temporary data stored in registers or shared memory.

#### 4.4 Interaction with the Autotuner

The autotuner can use PEA results as features for the ML pruner:
- **Allocation Density:** The number of allocations per basic block
- **Escape Ratio:** The ratio of escaping allocations to total allocations
- **Scalar Replacement Opportunities:** The number of allocations that can be scalar-replaced

These features help the autotuner predict which configurations will yield the best performance. For example:
- High allocation density → prioritize passes that reduce allocations (e.g., loop unrolling to enable more scalar replacement)
- High escape ratio → prioritize passes that reduce pointer aliasing (e.g., escape analysis-driven inlining)

---

### 5. Integration with Existing Architecture

PEA integrates seamlessly with the existing compiler architecture:

**Heterogeneous IR Extensions:**
- Add `Allocation` nodes
- Add `Escape` edges
- Add `Materialization` nodes

**Pass Pipeline Integration:**
- PEA runs after the borrow checker (which provides liveness information)
- PEA runs before the backend (which benefits from reduced allocations)
- PEA is a concurrent pass (function-level parallelism)

**Autotuner Integration:**
- PEA results are used as features for the ML pruner
- The autotuner can tune PEA parameters (e.g., threshold for scalar replacement)

**Backend Integration:**
- The backend receives IR with minimal allocations
- Register allocation benefits from scalar-replaced values (more registers available)
- Instruction selection can use scalar-replaced values directly (no load/store)

---

### 6. Performance Impact

**Macro Expansion:**
- Before PEA: $O(N)$ heap allocations per macro invocation
- After PEA: $O(1)$ heap allocations per macro invocation
- Speedup: 2-5x for macro-heavy code

**Runtime:**
- Before PEA: $O(N)$ heap allocations per compilation phase
- After PEA: $O(1)$ heap allocations per compilation phase
- Speedup: 1.5-3x for the compiler itself

**Kernels:**
- Before PEA: Allocations fail or are very slow
- After PEA: Zero allocations, all data in registers/shared memory
- Speedup: 10-100x for allocation-heavy kernels

---

### Summary

PEA is a **unifying optimization** that applies across the entire compiler:
1. **Frontend:** Eliminates allocations in macro expansion and name resolution
2. **Runtime:** Eliminates allocations in the compiler's own execution engine
3. **Backend:** Eliminates allocations in GPU kernels and other accelerator code

By integrating PEA into the Heterogeneous IR and applying it at every level, we achieve a compiler that is **allocation-free on the hot paths**, maximizing performance across the entire stack.


This is the most complex component of the compiler. Rust’s grammar is context-sensitive, its macro system is Turing-complete, and its type system involves higher-order logic.

To achieve **max perf** and **zero dependencies**, we cannot use standard parser generators (like Yacc/Bison) or off-the-shelf type inference libraries. We must build a custom, concurrent, arena-based engine from scratch.

Here is the complete technical design for the **Rust Frontend**.

---

### 1. The Lexical Layer: Token Trees (Not Streams)

Standard lexers produce a flat stream of tokens. Rust requires **Token Trees** because macros operate on nested structures (parentheses, brackets, braces) that must be preserved as trees, not flattened.

**Data Structure:**
```rust
pub enum TokenTree {
    Token(Token),           // A single token (identifier, literal, keyword)
    Delimited(Span, Delim, Vec<TokenTree>), // A group: (), [], or {}
}
```

**Max-Perf Implementation:**
*   **Interning:** All identifiers and string literals are interned into a global, lock-free hash map. We store `SymbolId` (u32) in the AST, not the string itself.
*   **Arena Allocation:** Token trees are allocated in a thread-local bump allocator. We never `malloc` individual tokens.
*   **Span Encoding:** Spans (source locations) are encoded as a 32-bit integer (offset + length) or a reference to a `SourceMap` entry to save memory.

---

### 2. The Parsing Engine: Recursive Descent with Speculation

Rust’s grammar is ambiguous (e.g., `Foo < Bar` could be a comparison or a generic type). A standard LALR parser cannot handle this. We use **Recursive Descent** with **Semantic Feedback** and **Speculative Parsing**.

**The Mechanism:**
1.  **Semantic Feedback:** When the parser sees `<`, it queries the **Name Resolution** table: "Is `Foo` a type or a value?"
    *   If `Foo` is a known type → Parse as Generic.
    *   If `Foo` is a known value → Parse as Comparison.
    *   If `Foo` is unknown → **Speculative Parse**.
2.  **Speculative Parsing:** The parser forks its state (snapshots the token stream position and variable bindings). It attempts to parse as a Generic. If it fails (e.g., no matching `>`), it backtracks to the snapshot and parses as a Comparison.

**Concurrency Model:**
*   **Crate-Level Parallelism:** We parse the dependency graph bottom-up. All crates at the same depth in the dependency tree are parsed concurrently.
*   **Module-Level Parallelism:** Within a crate, we perform a "Shallow Parse" first (identifying module boundaries and `use` statements). Once imports are resolved, we deep-parse the bodies of independent modules in parallel.

---

### 3. The Macro Expansion Engine: Hygiene & Sandboxing

Macros are the hardest part of the frontend. They can generate arbitrary code, including new types and functions that affect parsing.

**The Architecture:**
1.  **Eager Expansion:** We expand macros *during* parsing. When the parser encounters a macro invocation, it pauses, expands the macro, and inserts the resulting Token Tree back into the stream.
2.  **Hygiene (Syntax Contexts):** To prevent macro-generated variables from clashing with user variables, every identifier carries a `SyntaxContext` (a chain of macro expansion IDs).
    *   `x` (user) has context `[]`.
    *   `x` (from macro M) has context `[M]`.
    *   They are distinct symbols even if they look the same.
3.  **Proc Macro Sandboxing:** Procedural macros (written in Rust) are executed in a **sandboxed environment** (either a separate process via IPC or a WASM runtime). This prevents a buggy macro from crashing the compiler or executing malicious code.

---

### 4. Name Resolution: The DefId Graph

The parser produces an AST with "string names". Name Resolution converts these strings into **DefIds** (Definition IDs), which are stable, unique integers.

**The DefId Structure:**
```rust
pub struct DefId {
    pub krate: CrateId,    // u32: Which crate?
    pub index: DefIndex,   // u32: Index within the crate's definition table
}
```

**The Algorithm:**
1.  **Build the Skeleton:** We traverse the AST and assign a `DefId` to every item (function, struct, trait, module).
2.  **Resolve Imports:** We process `use` statements to link aliases to their target `DefId`.
3.  **Resolve Paths:** We traverse the AST again. Every path (e.g., `std::vec::Vec`) is resolved to a specific `DefId`.
    *   If a path is ambiguous (e.g., a type and a function have the same name), we record *both* possibilities in a "Resolution Ambiguity" list to be solved during Type Inference.

**Concurrency:**
*   Name resolution is inherently sequential *within* a module (due to scoping rules), but **independent modules** can be resolved in parallel once the crate skeleton is built.

---

### 5. Type Inference: Unification & Constraints

Rust uses **Hindley-Milner** inference extended with **Traits** and **Associated Types**. This is not just "guessing types"; it is solving a system of equations.

**The Core Data Structure: Inference Table (Union-Find)**
We use a **Union-Find** data structure to track type variables.
```rust
pub struct InferenceTable {
    // parent[i] is the parent of node i in the disjoint set forest
    parent: Vec<u32>,
    // rank[i] is the rank of node i (for path compression)
    rank: Vec<u8>,
    // value[i] is the concrete type if known, or None if it's a variable
    value: Vec<Option<Ty>>,
}
```

**The Algorithm:**
1.  **Variable Creation:** Every unknown type (e.g., `let x = ...;`) creates a fresh type variable $T_x$.
2.  **Constraint Generation:** As we traverse the AST, we generate constraints:
    *   `x = 5` $\implies T_x = \text{Int}$
    *   `x + y` $\implies T_x : \text{Add}<T_y>$ (Trait Bound)
    *   `foo(x)` $\implies T_x = \text{ParamTypeOf}(foo)$
3.  **Unification:** We solve constraints by merging type variables.
    *   If $T_x = \text{Int}$ and $T_x = T_y$, then $T_y = \text{Int}$.
    *   This is done via the `union` operation in the Union-Find table.

**Max-Perf Optimization:**
*   **Path Compression:** The Union-Find uses path compression to keep `find` operations near $O(1)$.
*   **Lazy Normalization:** We don't fully normalize types (e.g., resolving associated types) until absolutely necessary, to avoid infinite loops in cyclic trait definitions.

---

### 6. Trait Resolution: The Logic Engine

This is the "Search" phase. We must prove that a type satisfies a trait (e.g., `Vec<T> : Clone`).

**The Selection Context:**
The trait solver is a **logic program evaluator**. It searches for "Impls" (implementations) that match the goal.

**The Algorithm (Recursive with Memoization):**
1.  **Candidate Assembly:** Given a goal `T : Trait`, find all `impl Trait for ...` in the crate and dependencies that *could* match `T`.
2.  **Winnowing:** Filter out candidates that definitely don't match (e.g., `impl Clone for String` doesn't match `Vec<T>`).
3.  **Confirmation:** For the remaining candidates, recursively check if their *where clauses* are satisfied.
    *   *Example:* To prove `Vec<T> : Clone`, we must prove `T : Clone`.
4.  **Memoization (The Cache):** We cache the result of every trait query (`(Type, Trait) -> Success/Failure`). This prevents exponential blowup in complex generic code.

**Handling Ambiguity:**
If multiple impls match (e.g., a blanket impl `impl<T> Trait for T` and a specific impl `impl Trait for String`), we use **Specialization Rules** (if enabled) or report an ambiguity error.

---

### 7. Pattern Matching: Exhaustiveness & Usefulness

Rust requires `match` statements to be exhaustive (cover all cases) and useful (no unreachable arms).

**The Algorithm:**
1.  **Construct the Decision Tree:** We convert the `match` arms into a decision tree of "constructors" (e.g., `Some`, `None`, `Ok`, `Err`).
2.  **Usefulness Check:** For each arm, we check if there is any value that would reach this arm. If not, it's unreachable (Warning).
3.  **Exhaustiveness Check:** After processing all arms, we check if the decision tree covers the entire type space. If not, we compute the "missing" patterns and report an error.

**Max-Perf:**
*   This is essentially a **small theorem prover**. We optimize it by specializing the algorithm for common types (Booleans, Enums, Options) to avoid the overhead of the general algorithm.

---

### 8. The Lowering Pass: Rust HIR → Heterogeneous IR

The frontend produces the **Rust HIR** (High-Level IR), which is typed, resolved, and borrow-checked. The backend needs the **Heterogeneous IR** (Compute, Memory, Control nodes).

**The Transformation:**
1.  **Monomorphization:** We instantiate all generic functions. `foo<T>` becomes `foo_i32`, `foo_f64`, etc.
2.  **Closure Conversion:** Closures are converted into explicit structs with a `call` method.
3.  **Node Mapping:**
    *   `let x = a + b;` → `Compute(Add, a, b)`
    *   `vec.push(x);` → `Call(VecPush, vec, x)`
    *   `if cond { ... }` → `Control(Branch, cond, ...)`
4.  **Allocation Tagging:** We tag every `Box::new`, `Vec::new`, etc., with an `AllocationSiteId`. This is the hook for the backend's **Partial Escape Analysis (PEA)**.

---

### 9. Concurrency Model Summary

*   **Lexing/Parsing:** Parallel by Crate/Module.
*   **Name Resolution:** Parallel by Module (after skeleton build).
*   **Type Inference:** Parallel by Function (mostly). *Note: Global type inference (e.g., for trait solving) requires a synchronized "Obligation Forest", but individual function bodies can be inferred concurrently.*
*   **Trait Resolution:** Parallel by Obligation (using the memoization cache).
*   **Pattern Matching:** Parallel by Match Expression.

This frontend design provides the **semantic correctness** of `rustc` with the **performance** of a custom, concurrent, zero-allocation engine. It produces the perfectly typed, resolved, and borrow-checked Heterogeneous IR that the Autotuner and Backend require.

## The Runtime: Zero-Dependency Foundation (No Code)

The runtime is the minimal, self-contained foundation that allows generated code to execute without linking to `libc`, `std`, or any external dependencies. It provides the essential types, traits, and operations that Rust code requires.

---

### 1. Core Primitive Types

The runtime defines the fundamental types that map directly to hardware primitives.

**Integer Types:**
The runtime provides signed integers (8-bit, 16-bit, 32-bit, 64-bit, 128-bit) and unsigned integers (8-bit, 16-bit, 32-bit, 64-bit, 128-bit), plus pointer-sized integers (isize, usize).

These types are not defined in runtime code; they are **compiler intrinsics**. The compiler recognizes these type names and maps them directly to the Heterogeneous IR's primitive type system, which then maps to machine register classes. For example, a 32-bit signed integer becomes a value in the GPR (General Purpose Register) class with a 32-bit width annotation.

**Floating-Point Types:**
The runtime provides 32-bit IEEE 754 single-precision floats and 64-bit IEEE 754 double-precision floats. These map to the FPR (Floating Point Register) class in the Heterogeneous IR, which then maps to SIMD register files (XMM/YMM/ZMM on x86, V registers on ARM).

**Other Primitives:**
The runtime provides booleans (8-bit, values constrained to 0 or 1), characters (32-bit Unicode scalar values), and the unit type (zero-sized, used for functions that return nothing).

---

### 2. Core Traits

Traits define behavior. The runtime provides the most fundamental traits that all types must implement.

**Marker Traits (Zero-Cost):**
The runtime defines marker traits that carry no methods but provide semantic information to the compiler:
- **Copy**: Indicates a type can be duplicated by bitwise copy (no destructor needed)
- **Clone**: Indicates a type can be explicitly cloned (requires a clone method)
- **Sized**: Indicates a type has a known size at compile time
- **Send**: Indicates a type can be transferred across thread boundaries
- **Sync**: Indicates a type can be shared across thread boundaries

These traits are checked during the borrow checker and type inference phases. The compiler uses them to determine whether certain optimizations are safe (e.g., can we pass this by value? can we send this to another thread?).

**The Drop Trait:**
The Drop trait defines a single method that is called when a value goes out of scope. The compiler automatically inserts calls to drop at the end of scopes. The runtime provides default implementations for primitive types (no-op) and requires custom implementations for types with resources (e.g., file handles, network connections).

**Operator Traits:**
The runtime defines traits for all arithmetic and bitwise operators: Add, Sub, Mul, Div, Rem, Neg, Not, BitAnd, BitOr, BitXor, Shl, Shr. Each trait defines a method that takes two operands and returns a result.

These traits are implemented for primitive types using compiler intrinsics. For example, the Add trait for 32-bit integers is implemented by generating an ADD instruction in the backend. The compiler recognizes these trait implementations and lowers them directly to machine instructions, bypassing the normal function call mechanism.

---

### 3. Basic Collections

The runtime provides minimal implementations of essential data structures.

**Option<T>:**
Option is an enumeration with two variants: None (no value) and Some(T) (contains a value of type T). It provides methods for checking whether a value is present (is_some, is_none), extracting the value (unwrap, which panics if None), and transforming the value (map, which applies a function to the contained value if present).

Option is implemented as a tagged union: a discriminant byte (0 for None, 1 for Some) followed by the value (if Some). The compiler optimizes Option<Pointer> to use null as the None representation, eliminating the discriminant byte entirely (this is the "null pointer optimization").

**Result<T, E>:**
Result is an enumeration with two variants: Ok(T) (success value) and Err(E) (error value). It provides methods for checking the variant (is_ok, is_err), extracting the value (unwrap, which panics if Err), and transforming the value (map, map_err).

Result is implemented as a tagged union similar to Option, but with a larger discriminant to distinguish between Ok and Err.

**Box<T> (Heap Allocation):**
Box represents a heap-allocated value. It contains a pointer to the allocated memory. When a Box goes out of scope, it deallocates the memory and calls the destructor of the contained value.

Box is implemented as a single pointer-sized value. The runtime's allocator is called to allocate memory when the Box is created, and to deallocate memory when the Box is dropped.

**Vec<T> (Dynamic Array):**
Vec is a growable array. It contains a pointer to the buffer, the current length, and the capacity. It provides methods for adding elements (push), removing elements (pop), accessing elements by index, and querying the length and capacity.

Vec is implemented as three pointer-sized values: the buffer pointer, the length, and the capacity. When the length reaches the capacity, the buffer is reallocated to a larger size (typically doubling the capacity).

**RawVec<T> (Low-Level Buffer):**
RawVec is the internal buffer management structure used by Vec. It handles the raw pointer, capacity, and reallocation logic. It is not exposed to users; it is an implementation detail of Vec.

---

### 4. Iterator Trait

The iterator pattern is central to Rust. The runtime provides the core Iterator trait, which defines a single method: next, which returns an Option containing the next item (or None if the iterator is exhausted).

The Iterator trait also provides many default methods that are implemented in terms of next: map (transform each element), filter (keep only elements that satisfy a predicate), fold (accumulate a value by applying a function to each element), collect (convert the iterator into a collection), and many others.

These default methods are implemented as zero-sized types (ZSTs) that wrap the underlying iterator. For example, the map method returns a Map struct that contains the original iterator and the mapping function. The Map struct itself has zero size; it only exists at compile time to guide the compiler's code generation.

Basic iterator implementations are provided for slices (Iter, which yields references to elements), ranges (Range, which yields integers), and other core types.

---

### 5. Software Intrinsics (compiler-rt)

For operations that the hardware doesn't support directly, the runtime provides software implementations. These are called "intrinsics" because the compiler recognizes them and may replace them with hardware instructions if the target supports them.

**128-bit Integer Arithmetic:**
The runtime provides software implementations of 128-bit division and modulo, which are not supported by most hardware. These use shift-and-subtract algorithms that operate on 64-bit limbs.

**Floating-Point Soft-Float:**
For targets without a floating-point unit (FPU), the runtime provides software implementations of floating-point arithmetic: addition, subtraction, multiplication, division, and conversions between integer and float types. These use integer arithmetic to simulate IEEE 754 operations.

**Memory Operations:**
The runtime provides optimized implementations of memcpy (copy memory), memset (fill memory with a byte value), and memcmp (compare memory). These use SIMD instructions when available to process multiple bytes in parallel.

**Atomic Operations:**
For targets without hardware atomic instructions, the runtime provides software implementations using spinlocks. These are much slower than hardware atomics but provide the same semantics.

---

### 6. Memory Allocator

The runtime provides a minimal, zero-dependency memory allocator.

**The Allocator Trait:**
The allocator trait defines methods for allocating, deallocating, growing, and shrinking memory blocks. Each method takes a Layout, which specifies the size and alignment requirements of the allocation.

**Layout:**
A Layout is a pair of values: the size (in bytes) and the alignment (in bytes, must be a power of two). The allocator ensures that the returned pointer is aligned to the specified alignment.

**Global Allocator:**
The global allocator is the default allocator used by Box, Vec, and other collections. It delegates to a platform-specific allocation function.

**Platform-Specific Allocation:**
For bare-metal targets (no operating system), the allocator uses a simple bump allocator: a fixed-size array (e.g., 1MB) with a pointer that advances as memory is allocated. Deallocation is a no-op; memory is only reclaimed when the program exits.

For OS targets, the allocator calls system functions (e.g., malloc/free on Unix, HeapAlloc/HeapFree on Windows). However, since we're building a zero-dependency compiler, we typically target bare-metal or provide our own minimal OS interface.

---

### 7. Panic & Exception Handling

The runtime provides a minimal panic mechanism.

**Panic Implementation:**
When a panic occurs (e.g., unwrap on None, index out of bounds), the runtime calls a panic handler. The panic handler prints the panic message (if a console is available) and then either aborts the program or unwinds the stack.

**Unwind Implementation (Zero-Cost Exceptions):**
For targets that support unwinding, the runtime uses table-based exception handling. The compiler generates unwind tables (in the .eh_frame section) that describe how to restore the stack and call destructors as the stack unwinds.

The unwind process works as follows:
1. The panic handler calls the platform-specific unwind function
2. The unwind function reads the .eh_frame section to find the current function's unwind information
3. The unwind information describes how to restore the stack pointer, frame pointer, and other registers
4. As the stack unwinds, the runtime calls destructors for any values that go out of scope
5. The unwind continues until it reaches a catch point (e.g., a catch_unwind call) or the top of the stack

If unwinding is disabled (e.g., for bare-metal targets or for performance), the program simply aborts on panic.

---

### 8. Integration with the Compiler

The runtime integrates with the compiler's lowering passes in several ways:

**Type Mapping:**
Primitive types in Rust (i32, f64, etc.) are recognized by the compiler and mapped directly to the Heterogeneous IR's primitive type system. The backend then maps these to machine register classes.

**Allocation Tagging:**
When the compiler encounters a Box::new or Vec::new call, it tags the allocation site in the Heterogeneous IR. The backend's Partial Escape Analysis (PEA) pass analyzes these allocations and may scalar-replace them (convert heap allocations to stack allocations or eliminate them entirely).

**Intrinsic Lowering:**
Operator trait implementations for primitive types are recognized by the compiler and lowered directly to machine instructions. For example, the Add trait for i32 is lowered to a single ADD instruction, bypassing the normal function call mechanism.

**Drop Glue:**
For types with Drop implementations, the compiler generates "drop glue": a function that calls all destructors in the correct order. The drop glue is inserted into the Heterogeneous IR as Drop nodes, which the backend schedules appropriately.

---

### 9. Concurrency & Performance

**Zero-Allocation Design:**
The runtime itself is designed to minimize allocations. Iterator adapters (Map, Filter, etc.) are zero-sized types (ZSTs) that don't allocate; they only exist at compile time to guide code generation. Collections use the global allocator, but PEA can eliminate many allocations.

**Inlining:**
Critical runtime functions (e.g., Option::unwrap, Result::unwrap) are marked for aggressive inlining. The compiler inlines these to eliminate function call overhead, especially in hot loops.

**SIMD Optimization:**
Memory operations (memcpy, memset, memcmp) are optimized using SIMD instructions. The backend's autotuner selects the best implementation for the target hardware (e.g., using AVX2 for 256-bit vector operations on x86).

---

### Summary

The runtime provides:
1. **Primitive types** that map directly to hardware register classes
2. **Core traits** that define fundamental behavior and enable compiler optimizations
3. **Basic collections** (Option, Result, Box, Vec) with minimal, allocation-efficient implementations
4. **Iterator trait** for functional programming patterns, implemented as zero-sized types
5. **Software intrinsics** for operations the hardware doesn't support (128-bit math, soft-float, memory operations)
6. **Memory allocator** for heap allocations, with platform-specific implementations
7. **Panic handling** for error recovery, with optional zero-cost unwinding

This runtime is **zero-dependency**, **self-contained**, and **optimized for performance**. It allows generated code to run on bare-metal targets without linking to external libraries, while still providing the full power of Rust's type system and standard library patterns.

## The ABI: Contract with the OS and Hardware

The Application Binary Interface (ABI) is the immutable, mathematically precise contract between the compiled machine code and the execution environment (the OS, the hardware, and external libraries). If the compiler violates even a single byte of this contract, the program will crash, corrupt memory, or fail to link. 

Because we are building a zero-dependency, max-performance compiler, the ABI design must be rigorously defined, highly optimized, and deeply integrated into the backend's instruction selection and register allocation phases.

Here is the complete, code-free architectural design for the ABI.

---

### 1. Calling Conventions: The Register and Stack Protocol

The calling convention dictates exactly how data moves between a caller and a callee. We must define two distinct conventions: the **Internal Rust ABI** (optimized for speed) and the **External C ABI** (strictly compliant with OS standards).

**The External C ABI (System V AMD64 / Windows x64 / ARM64):**
This must perfectly match the target OS's expectations. 
- **Integer and Pointer Arguments:** The first several arguments are passed in specific General Purpose Registers (e.g., RDI, RSI, RDX on Linux; RCX, RDX, R8 on Windows). Remaining arguments are pushed onto the stack in right-to-left order.
- **Floating-Point and Vector Arguments:** Passed in specific SIMD registers (e.g., XMM0 through XMM7). 
- **Return Values:** Small values (up to 16 bytes) are returned in registers (RAX/RDX or XMM0/XMM1). Large structs are returned via a hidden pointer argument (the caller allocates space and passes the address as the first hidden parameter).
- **Variadic Functions:** For functions like printf, the ABI requires a specific register (e.g., AL on System V) to contain the number of vector registers used, and variadic arguments must be passed on the stack to allow sequential reading.

**The Internal Rust ABI:**
Because Rust controls both the caller and the callee, we can break C ABI rules for massive performance gains.
- **Aggregate Passing:** Small structs and enums are passed in registers, even if the C ABI would force them onto the stack.
- **Zero-Sized Types (ZSTs):** Arguments and return values of size zero are completely erased from the calling convention. They consume zero registers and zero stack space.
- **Niche Optimization:** Types like Option<NonNull<T>> are passed as a single pointer register, utilizing the null pointer niche to avoid passing a separate discriminant byte.

**GPU and Device ABIs:**
For heterogeneous targets, the calling convention changes entirely. Kernel arguments are not passed on a stack. They are marshaled into a contiguous **Argument Buffer** placed in Constant Memory or Global Memory. The hardware dispatcher reads this buffer and populates the compute unit's argument registers before launching the kernel.

---

### 2. Stack Frame Layout and Memory Model

The stack frame is the localized memory environment for a single function invocation. The compiler must construct the prologue (setup) and epilogue (teardown) with extreme precision.

**Frame Components:**
A standard stack frame consists of:
1. **Outgoing Arguments Area:** Space reserved for arguments passed to subsequent function calls.
2. **Local Variable Area:** Space for variables that cannot fit in registers or are explicitly addressed (e.g., borrowed references).
3. **Saved Registers Area:** Space to spill callee-saved registers that the function modifies.
4. **Saved Frame Pointer:** The caller's frame pointer, saved to allow stack walking.
5. **Return Address:** Pushed onto the stack automatically by the hardware Call instruction.

**Stack Alignment:**
Modern hardware requires strict stack alignment for SIMD instructions. The stack pointer must be aligned to a 16-byte boundary (or 32-byte for AVX-512) immediately before any Call instruction is executed. The compiler's prologue must dynamically calculate the required padding to maintain this alignment, taking into account the return address pushed by the caller.

**The Red Zone:**
On System V architectures, the 128 bytes immediately below the stack pointer are the "Red Zone." Leaf functions (functions that do not call other functions) can use this space for temporary variables without adjusting the stack pointer. This saves an instruction in the prologue and epilogue. The backend must track Red Zone usage and forbid its use in interrupt handlers or functions that receive signals.

**Stack Clash Protection:**
To prevent attackers from jumping over the stack guard page, the compiler must generate "stack probing" code. If a function allocates more than one page of stack space, the prologue must iteratively touch each page (e.g., writing a zero to the page boundary) before moving the stack pointer down. This ensures the hardware page fault triggers correctly.

---

### 3. Exception Handling and Zero-Cost Unwinding

Rust uses a "zero-cost" exception model. The normal execution path pays absolutely no performance penalty for the possibility of a panic. The cost is entirely shifted to the rare event of an actual panic, and to the binary size of the unwind tables.

**Call Frame Information (CFI):**
The compiler must generate metadata that describes the exact state of the machine at every single instruction boundary. This metadata (stored in the .eh_frame section on Linux/ELF or .xdata on Windows/PE) maps an instruction address to:
- The offset of the return address.
- The location of the saved frame pointer.
- The locations of all callee-saved registers.

**Personality Routines and Landing Pads:**
When a panic occurs, the OS unwinder walks the stack using the CFI. At each frame, it calls a "Personality Routine" generated by the compiler. The personality routine examines the panic payload and the current instruction pointer to decide:
1. **Cleanup:** Does this frame have destructors (Drop implementations) that need to run? If so, the unwinder redirects execution to a "Landing Pad" (a hidden block of code generated by the compiler that runs the destructors and then resumes unwinding).
2. **Catch:** Does this frame have a catch_unwind block? If so, the unwinder restores the state and resumes execution inside the catch block.
3. **Propagate:** If neither, the unwinder uses the CFI to restore the caller's registers and moves up the stack.

**Integration with the Backend:**
The instruction scheduler must treat Landing Pads as separate, cold basic blocks. The DP-on-DAG instruction selector will naturally assign them low-priority patterns, and the autotuner will ensure they do not bloat the hot path.

---

### 4. Thread-Local Storage (TLS) Models

Thread-Local Storage allows global variables to have a unique instance per thread. The ABI defines how the compiled code accesses these instances. There are four models, ranging from fastest to most flexible.

**Local-Exec (The Zero-Dependency Standard):**
Since our compiler produces statically linked, zero-dependency binaries, we primarily use Local-Exec. The TLS variable is placed in the main executable's TLS segment. The compiler generates a single instruction that reads from a fixed offset relative to the hardware Thread Pointer register (e.g., the FS segment register on x86-64, or the TPIDR_EL0 register on ARM64). This requires no dynamic linking and executes in a single cycle.

**Initial-Exec and Local-Dynamic:**
Used if we ever support shared libraries. These involve reading a pointer from a Global Offset Table (GOT) to find the TLS offset, adding a slight runtime overhead.

**Global-Dynamic:**
The most flexible but slowest model. It involves calling a runtime helper function to resolve the TLS address. Since we are zero-dependency, the compiler will reject this model or provide a highly optimized, inline software implementation of the resolver.

---

### 5. Symbol Visibility, Name Mangling, and FFI

The ABI dictates how symbols are named and exposed to the outside world.

**Name Mangling:**
Rust's internal ABI uses a complex mangling scheme (the v0 mangling format) that encodes the crate hash, module path, generic type parameters, and disambiguators into the symbol name. This allows the linker to distinguish between `foo<i32>` and `foo<f64>` without collisions. The External C ABI strictly forbids mangling; the compiler must strip all mangling and emit the exact string provided by the programmer.

**Symbol Visibility:**
Every symbol is assigned a visibility attribute:
- **Default (Exported):** The symbol is visible to the linker and can be called from other object files.
- **Hidden (Internal):** The symbol is strictly local to the current translation unit. The linker is forbidden from exporting it. This is critical for the compiler's Link-Time Optimization (LTO) and Identical Code Folding (ICF), as hidden symbols can be safely duplicated, merged, or deleted without breaking external contracts.

**Foreign Function Interface (FFI) Marshaling:**
When crossing the boundary between Rust and C (or the OS kernel), the types must be translated. 
- Rust's `bool` is an 8-bit type constrained to 0 or 1. C99's `_Bool` has different promotion rules. The compiler must insert truncation or masking nodes in the Heterogeneous IR.
- Rust's `char` is a 32-bit Unicode scalar. C's `char` is 8 bits. The FFI boundary requires explicit conversion nodes.
- Pointers must be annotated with their mutability and alignment guarantees, which the borrow checker has already verified, allowing the backend to emit stricter, faster memory access instructions.

---

### 6. Integration with the Max-Perf Engine

The ABI is not an afterthought; it is deeply wired into the compiler's optimization engine.

**Register Allocation (MCF):**
The Min-Cost Multi-Commodity Flow register allocator receives hard constraints from the ABI. Argument registers are pre-colored. If a function takes three integer arguments, the allocator's flow network forces the first three virtual registers to map to the exact physical registers mandated by the calling convention.

**Instruction Selection (DP-on-DAG):**
The instruction selector uses ABI rules to fold operations. For example, if a function returns a small struct, the selector knows it must place the fields into the specific return registers (e.g., RAX and RDX), potentially selecting a single instruction that writes to both.

**The Autotuner:**
The autotuner searches for ABI-related optimizations. 
- Should this function omit the Frame Pointer to save a register and an instruction, or keep it to allow external profilers to work?
- Should this leaf function utilize the Red Zone to save stack adjustment overhead?
- Should the stack probing interval be adjusted based on the function's maximum stack depth?

By treating the ABI as a first-class, mathematically modeled component of the Heterogeneous IR, the compiler ensures that every generated binary perfectly satisfies the hardware and OS contracts while extracting every possible cycle of performance.


## 4. Incremental Compilation & Caching

To achieve interactive compile times for massive Rust codebases, the compiler cannot re-evaluate the entire program on every keystroke or file save. We must build a **Content-Addressable, Fully Incremental, Distributed Caching Architecture** that spans every phase of the compiler, from the lexer to the final machine code emission, including the highly expensive Autotuner.

### The Content-Addressable Storage (CAS) Model

The foundation of the caching system is a Content-Addressable Storage model, identical in principle to Git's object database, but optimized for compiler artifacts. 

Every artifact produced by the compiler (an AST node, a resolved HIR function, a lowered LIR block, a final object file, and crucially, an Autotuner configuration) is serialized and hashed using a fast, cryptographic hash function (like BLAKE3). The artifact is then stored in a local cache directory, named by its hash. If the hash matches an existing file, the artifact is already cached, and we skip the computation entirely.

### Frontend Incrementality: The Dependency Graph

The parser and name resolver do not process the entire crate as a single monolithic block. They operate on a fine-grained **Module Dependency Graph**.

When a source file changes, the compiler computes the hash of the new file content. If the hash matches the cached AST, the frontend stops immediately. If it differs, the parser re-parses only that file. 

The Name Resolution phase maintains a concurrent graph of DefIds (Definition IDs). If a function's signature changes, the compiler identifies all downstream functions that call it or reference its types via this graph. Only those specific downstream functions have their HIR invalidated and re-resolved. Unaffected functions retain their cached HIR.

### Middle-End (IR) Incrementality: Pass Invalidation

The Heterogeneous IR and the optimization passes are fully incremental. The compiler maintains a **Pass Dependency Matrix** that tracks which analyses and transformations depend on which IR properties.

If a function's HIR is modified, the compiler invalidates its MIR. However, it does not blindly re-run all 170 optimizations. It checks the Pass Dependency Matrix. For example, if only the body of a loop changed but the loop bounds and structure remained identical, passes like Loop Unrolling and Loop Tiling are skipped, and the compiler jumps directly to Data Flow passes like Dead Code Elimination.

**Zero-Copy IR Serialization:**
To make cache loading instantaneous, the Heterogeneous IR is serialized using a memory-mappable, zero-copy format. When the compiler loads a cached IR from disk, it simply calls the operating system's memory map function. The IR is instantly available in memory as native Rust structs without any deserialization or parsing overhead.

### Backend Incrementality and The Autotuner Cache

The backend is where the most significant compile-time savings occur, primarily through caching the Autotuner's results.

**Machine IR Caching:**
If a function's LIR hash matches the cache, the compiler skips instruction selection, register allocation, and scheduling entirely. It loads the cached Machine IR (or the final compiled object file) directly from disk.

**The Autotuner Cache (The Holy Grail):**
The Autotuner's 4-tier evaluation funnel (especially Tier 3 silicon validation) takes seconds or minutes. We must cache its results aggressively. 
The cache key for the Autotuner is a composite hash of:
1. The Function's LIR Hash.
2. The Target Hardware Fingerprint (exact CPU model, cache sizes, SIMD capabilities).
3. The Compiler Version.

If this composite key matches the cache, the compiler instantly loads the cached **Plackett-Luce logits (pass ordering), the optimal iteration counts, and the exact factor values (tile sizes, unroll factors)**. It bypasses the ML pruner and the entire evaluation funnel, applying the known-optimal configuration directly. This reduces the compilation of hot, heavily-tuned functions from minutes to microseconds on subsequent builds.

### Distributed Caching and Concurrency

For large engineering teams, the local cache is insufficient. The compiler integrates with a **Distributed CAS** (e.g., a remote Redis cluster or an S3-compatible object store). 

Before compiling a function, the compiler queries the remote cache. If a colleague has already compiled this exact function for this exact target, the object file is downloaded over the network, completely eliminating the local compute cost.

The entire caching infrastructure is **lock-free**. Multiple worker threads compiling different functions concurrently can query and insert into the cache using atomic operations and memory-mapped files, ensuring the cache never becomes a bottleneck.

---

## 5. Diagnostics & Developer Experience

A max-performance compiler is useless if it takes ten seconds to report a simple type error. The diagnostics engine must be instantaneous, highly accurate, and capable of mapping the compiler's deeply nested, multi-stage IR transformations back to the exact line of source code the programmer wrote.

### The Span and Provenance Chain

Every single node in the AST, HIR, MIR, LIR, and Machine IR carries a **Span**—a lightweight pointer to the exact byte range in the original source file.

However, the compiler heavily transforms code. It expands macros, monomorphizes generics, generates drop glue, and performs partial escape analysis. To track errors through these transformations, we use a **Provenance Chain**. 

When the compiler generates synthetic code (e.g., a destructor call inserted by the borrow checker), the new IR node carries a Span that points to the original variable declaration, but it also carries a "Macro/Expansion Context" link. If an error occurs in the synthetic code, the diagnostic engine walks the provenance chain backward, generating a message like: *"Error in drop glue generated for variable 'x', which was defined at line 42, inside the expansion of macro 'my_macro' invoked at line 10."*

### The Concurrent Diagnostic Engine

Diagnostics are not formatted into strings during the compilation phase. String allocation and terminal I/O are slow and would block the concurrent compilation pipeline.

Instead, when a pass (like the Borrow Checker or Type Inference) encounters an error, it emits a **Structured Diagnostic Object**. This object contains the primary error code, the primary span, a list of secondary spans (for context), and structured help messages.

These objects are pushed into a **Lock-Free Concurrent Queue**. The compilation threads never stop to format or print errors. 

A dedicated, low-priority background thread (or the main thread at the very end of compilation) drains this queue. It sorts the errors by source file and line number, formats them into rich, multi-line, colorized terminal output, and prints them. This ensures that the compilation speed is entirely unaffected by the complexity of the error messages.

### Mapping the Autotuner and Backend to Source

When the backend fails (e.g., the Min-Cost Multi-Commodity Flow register allocator determines that a function has impossible register constraints and cannot be compiled without excessive spilling), the error originates deep in the Machine IR.

The diagnostic engine uses the Provenance Chain to map the failing Machine IR instruction back to the LIR, then to the MIR Sea of Nodes, then to the HIR, and finally to the original Rust source code. It tells the programmer exactly which line of code caused the register pressure to exceed the hardware limits, rather than presenting an opaque internal compiler error.

### Developer Experience (DX) Features

To provide a modern developer experience, the compiler operates in multiple modes:

**Watch Mode (The Daemon):**
The compiler runs as a persistent background daemon. It uses the operating system's file system notification APIs (inotify on Linux, FSEvents on macOS) to watch the source tree. When a file is saved, the daemon computes the file's hash, updates the Incremental Dependency Graph, invalidates only the affected caches, and recompiles the delta. Because the heavy lifting (Autotuner, Backend) is cached, the daemon can often output a new binary in under 50 milliseconds.

**Language Server Protocol (LSP) Integration:**
The compiler exposes a Query API specifically for IDEs. When a programmer types a character, the IDE sends a "check" request. The compiler uses its incremental frontend to parse the changed file, runs the type checker and borrow checker on just that file (using cached HIR for the rest of the crate), and returns the structured diagnostics to the IDE in milliseconds. It also answers queries like "What is the type of this expression?" or "Where is this trait implemented?" by directly querying the Name Resolution and Type Inference caches.

**IR Inspection and Debugging:**
For compiler developers and performance engineers, the compiler can emit the internal IR at any stage. By passing a specific flag, the compiler will dump the Heterogeneous IR, the Sea of Nodes, the LIR, and the final Machine IR in a human-readable, graphical format (like DOT graphs for control flow). This allows developers to visually inspect exactly how the Autotuner's Plackett-Luce model reordered the passes, or how the DP-on-DAG instruction selector folded a complex addressing mode.


## 6. The Compiler Driver & Build Integration

The Compiler Driver is the central orchestrator. It does not perform the heavy lifting of parsing or code generation; instead, it manages the session state, orchestrates the concurrent execution of the compilation pipeline, and interfaces with the external build ecosystem. To achieve maximum performance, the driver must be entirely non-blocking and rely on a highly optimized, memoized query system.

### The Session and Immutable Configuration

When the driver starts, it parses the command-line arguments and constructs a **Session** object. This object contains the target triple, optimization levels, codegen options, and file paths. 

Crucially, once the Session is constructed, it is **strictly immutable**. It is wrapped in an `Arc` (Atomically Reference Counted pointer) and shared across all worker threads. Because it never changes, threads can read session data without any locks, mutexes, or atomic contention. This eliminates a massive source of overhead in traditional compiler drivers.

### The Memoized Query System (The Backbone of Incrementality)

The driver does not execute passes in a rigid, linear sequence. Instead, it uses a **Demand-Driven Query System**. Every piece of data the compiler produces (the AST of a module, the type of an expression, the MIR of a function, the machine code of a block) is the result of a "Query".

When the backend needs the MIR of a function, it doesn't ask the driver to "run the MIR pass." It invokes the `mir_of(function_id)` query. The query system checks its cache:
1. If the query has been executed and its inputs haven't changed, it returns the cached result instantly (zero-copy, directly from the content-addressable store).
2. If the query is not cached, the system recursively invokes the queries it depends on (e.g., `typeck_of`, `borrowck_of`), executes the computation, caches the result, and returns it.

This architecture naturally enables incremental compilation. If a single line of code changes, only the queries that directly or transitively depend on that line are re-executed. The rest of the compilation graph remains frozen in the cache.

### Concurrency and Crate-Level Parallelism

The driver manages compilation at the **Crate Graph** level. When building a workspace with multiple crates, the driver constructs a dependency DAG of the crates. 

It uses a concurrent work-stealing scheduler to compile crates. If Crate A and Crate B both depend on Crate C, the driver waits for Crate C to finish its metadata generation, and then immediately dispatches Crate A and Crate B to two separate worker threads. They compile in parallel, completely independent of each other.

### Build System Integration and Cargo Interoperability

To be usable in the real world, the compiler must integrate seamlessly with Cargo and other build systems (Make, Ninja, Bazel).

**Drop-in `rustc` Replacement:**
The driver accepts the exact same command-line interface as the official Rust compiler. Build systems invoke our compiler binary exactly as they would invoke `rustc`. 

**Precise Dependency Tracking (`.d` files):**
Build systems need to know exactly which files trigger a recompilation. The driver tracks every single file read during compilation. This includes not just the primary source files, but also:
- Files included via `include!` or `include_str!` macros.
- The source code of procedural macros (which are compiled and executed dynamically).
- Environment variables read via `env!`.

At the end of compilation, the driver emits a Make-compatible `.d` file that lists the output object file and every single input file it depended on. This ensures that external build systems never miss a recompilation, nor do they recompile unnecessarily.

**Metadata Extraction (The `rlib` equivalent):**
When compiling a library, the driver must export the public API (types, traits, function signatures) so downstream crates can compile against it. The driver serializes this metadata into a highly compressed, memory-mappable format and appends it to the output object file. Downstream crates load this metadata directly via `mmap`, allowing the frontend to resolve types and traits without re-parsing the library's source code.

---

## 7. Self-Hosting & Bootstrap

Self-hosting is the ultimate validation of a compiler. It proves that the compiler is robust, complete, and capable of handling the extreme complexity of its own source code. However, achieving a **bit-for-bit reproducible** self-hosted build is one of the hardest engineering challenges in compiler design.

### The Bootstrap Stages

The bootstrap process occurs in distinct stages:

**Stage 0 (The Seed):**
A minimal, hand-written compiler (often in C, C++, or a very restricted subset of Rust). Its only job is to be correct and stable. It lacks the advanced optimizations, the autotuner, and the concurrent backend of our design. It is just smart enough to parse our compiler's source code and emit a working, unoptimized binary.

**Stage 1 (The Ascent):**
Stage 0 compiles the full source code of our max-perf, concurrent, autotuning Rust compiler. The resulting binary is Stage 1. It contains all the advanced features: the Sea of Nodes, the DP-on-DAG instruction selector, the MCF register allocator, and the Plackett-Luce autotuner.

**Stage 2 (The Mirror):**
Stage 1 compiles its own source code. The resulting binary is Stage 2. This binary is compiled with all the aggressive optimizations, autotuning, and zero-allocation passes that Stage 1 provides.

**Stage 3 (The Verification):**
Stage 2 compiles its own source code. The resulting binary is Stage 3. 

**The Golden Rule:** The binary output of Stage 2 and Stage 3 must be **mathematically, bit-for-bit identical**. If they differ by a single byte, the compiler is non-deterministic, and the build is considered failed.

### The Reproducibility Mandate

Achieving bit-for-bit reproducibility in a highly concurrent, ML-guided compiler requires eliminating every source of non-determinism.

**1. Deterministic Hashing (The Silent Killer):**
Standard hash maps (like Rust's default `HashMap` or `HashSet`) use randomized hashing to prevent HashDoS attacks. This means the iteration order of keys is different on every run. If the compiler iterates over a hash map to emit symbols or schedule passes, the output binary will change randomly. 
*Solution:* We replace all standard hash maps with deterministic alternatives (e.g., `FxHash` or `AHash` with a fixed, hardcoded seed). The iteration order is now strictly determined by the insertion order and the hash values, which are identical across runs.

**2. Deterministic Concurrency:**
In a work-stealing scheduler, the order in which threads pick up tasks is non-deterministic. If Thread A and Thread B both compute the type of two different expressions, and the final binary layout depends on which thread finished first, the build is non-reproducible.
*Solution:* The compiler's logical output must be independent of thread scheduling. We achieve this through the **Content-Addressable Cache** and **pure functions**. The final layout of the binary is determined by sorting symbols and sections by their deterministic hashes, not by the order in which threads finished compiling them.

**3. Deterministic Autotuning:**
The Autotuner uses a Machine Learning pruner and a Plackett-Luce model, both of which rely on random number generation to sample the search space.
*Solution:* The random number generator is never seeded with the system clock. Instead, it is seeded with a cryptographic hash of the function's LIR and the target hardware fingerprint. Given the exact same input function, the autotuner will explore the exact same search space, evaluate the exact same configurations, and select the exact same optimal parameters.

**4. Metadata Sanitization:**
Object files and executables often contain hidden metadata: build timestamps, absolute file paths, and random UUIDs. 
*Solution:* The Object File Emission layer strictly sanitizes the output. Timestamps are set to zero (or the Unix epoch). Absolute paths are stripped or replaced with relative, normalized paths. UUIDs are deterministically generated from the content hash.

### The Compiler's Own Runtime

Because we are building a "zero-dependency" compiler, the compiler itself cannot rely on bloated, external runtime libraries. It must use its own highly optimized runtime to execute.

When the compiler runs, it utilizes the custom **Compiler Runtime** we designed earlier:
- **Bump Allocators:** The frontend and middle-end allocate AST and IR nodes in massive, thread-local bump allocators, achieving allocation speeds of a single pointer increment.
- **Lock-Free Queues:** The diagnostic engine and the query system use lock-free concurrent queues to pass messages between threads without OS-level mutexes.
- **Direct Syscalls:** For file I/O and memory mapping, the runtime bypasses standard library wrappers and issues direct system calls (e.g., `mmap`, `read`, `futex`) to minimize overhead.

By compiling the compiler with itself, the compiler applies its own Partial Escape Analysis, Dead Code Elimination, and Autotuning to its own source code. Stage 2 is not just a self-hosted compiler; it is the most highly optimized, perfectly tuned version of the compiler possible for the host hardware.


## 8. Testing & Validation: The Ruthless Gauntlet

A compiler that crashes on edge cases is useless. A compiler that silently generates wrong code is catastrophic. To guarantee absolute correctness, we do not rely on unit tests. We build a **Distributed, Grammar-Aware, Coverage-Guided Fuzzing and Differential Validation Infrastructure** designed to subject the compiler to millions of adversarial inputs, breaking it continuously so that it never breaks in production.

### 1. The Grammar-Aware Fuzzing Engine (The Meat Grinder)

Standard byte-mutation fuzzers (like LibFuzzer) are ineffective for complex languages like Rust because 99.9% of random byte mutations result in immediate lexer errors, never reaching the deep, complex optimization passes. We require a **Semantic, Grammar-Aware Fuzzer**.

**Frontend Fuzzing (The Parser & Borrow Checker):**
The fuzzer operates on the Abstract Syntax Tree (AST) and Token Trees. It applies semantic mutations:
- Swapping variable declarations to trigger borrow checker conflicts.
- Injecting deeply nested, pathological macro expansions to test the hygiene context and stack limits.
- Generating valid but highly complex trait resolution graphs (e.g., deeply nested associated types, conflicting blanket implementations) to stress the logic engine.
- Injecting invalid UTF-8, zero-width characters, and malformed unicode to ensure the lexer never panics and always emits a structured diagnostic.

**Middle-End Fuzzing (The Heterogeneous IR):**
We bypass the frontend entirely and fuzz the IR directly. The fuzzer loads valid Heterogeneous IR from the corpus and applies destructive mutations:
- Deleting nodes to create dangling edges.
- Reversing data-flow edges to create cycles in the Sea of Nodes.
- Merging incompatible types to break the type checker.
- Creating impossible region constraints to stress the SCC condensation and bitset propagation in the borrow checker.

**Backend Fuzzing (Instruction Selection & Register Allocation):**
The fuzzer generates pathological LIR and Machine IR to break the backend:
- Generating functions with thousands of live variables to force the Min-Cost Multi-Commodity Flow (MCF) register allocator into extreme spilling scenarios.
- Creating massive, deeply nested DAGs to stress the DP-on-DAG instruction selector's memoization and chain-depth limits.
- Generating illegal register constraints (e.g., forcing a 64-bit float into a 32-bit integer register) to ensure the backend fails gracefully with an internal compiler error (ICE) rather than emitting silent garbage.

### 2. Differential Testing & The Oracle (The Truth Serum)

Fuzzing only tells us if the compiler crashes. To prove it generates *correct* code, we use **Differential Testing** against the reference implementation (`rustc` and LLVM).

**The Compilation Oracle:**
For every generated Rust program, we compile it with our compiler and with `rustc`. 
- If `rustc` accepts it and our compiler rejects it (or vice versa), we have a bug.
- If both accept it, we compare the generated assembly. While the assembly won't be identical (due to different optimization choices), the *structural semantics* must match. We use a structural diffing tool to ensure both compilers utilize the same calling conventions, stack layouts, and ABI rules.

**The Execution Oracle:**
We execute the binary generated by our compiler and the binary generated by `rustc`.
- We compare the standard output, standard error, and the exit code.
- We run the programs under a memory sanitizer (to detect use-after-free or buffer overflows) and compare the sanitizer reports.
- **Handling Non-Determinism:** We strictly isolate the execution environment. We disable ASLR, lock the CPU frequency, and mock the system clock. If the program relies on random number generation or threading, we inject deterministic seeds and a deterministic thread scheduler to ensure the execution order is identical across both binaries.

### 3. IR Invariant Verification (The Mathematical Shield)

We do not trust the optimization passes. We assume every pass is guilty until proven innocent. We implement a **Deterministic IR Verifier** that runs immediately after *every single pass* in the pipeline.

**The Verifier's Mandate:**
- **Sea of Nodes (MIR):** Verifies that every node's operands dominate the node. Verifies that Phi nodes are strictly placed at the beginning of basic blocks. Verifies that there are no cycles in the data-flow graph (unless explicitly marked as loop back-edges).
- **LIR:** Verifies that every virtual register is defined exactly once (SSA property). Verifies that all live ranges are contiguous and correctly bounded.
- **Machine IR:** Verifies that every instruction is valid for the target ISA. Verifies that physical registers do not conflict. Verifies that the stack frame is correctly aligned according to the ABI.

If the verifier detects a single violation, it immediately halts the compilation, dumps the exact state of the IR, and triggers a core dump. This ensures that a bug in Pass A never silently corrupts the IR for Pass B; the crash happens at the exact moment of corruption.

### 4. Automated Test Case Minimization (The Cleanup Crew)

When the fuzzer finds a crash, the input is often a 50,000-line, 10-megabyte monstrosity. A human cannot debug this. We require an **Automated, Parallelized Test Case Reducer**.

**The Delta Debugging Algorithm:**
The reducer takes the crashing input and the crash condition (e.g., "exits with SIGSEGV" or "emits an Internal Compiler Error"). It systematically removes chunks of the input:
1. It removes the first half of the file. Does it still crash? If yes, keep the first half.
2. It removes the second half. Does it still crash? If yes, keep the second half.
3. It recursively subdivides the remaining code, removing individual functions, then individual statements, then individual tokens.

**Semantic Reduction:**
Because removing arbitrary tokens often breaks the syntax (preventing the crash from reproducing), the reducer uses the compiler's own parser and AST manipulation tools to perform *semantic* reductions. It replaces complex expressions with simple literals, removes unused function arguments, and simplifies complex types to primitive integers, all while ensuring the program remains syntactically valid and the crash condition remains true.

The result is a 5-line, perfectly minimal reproduction case that can be added to the permanent regression test suite.

### 5. Distributed Fuzzing Infrastructure (The Army)

Running millions of fuzzing iterations on a single machine takes months. We build a **Distributed, Coverage-Guided Fuzzing Cluster**.

**Coverage-Guided Mutation:**
The fuzzer is not blind. It instruments the compiler to track which internal structures are touched during compilation:
- Which parser rules were hit?
- Which optimization passes modified the IR?
- Which DP-on-DAG instruction selection rules were evaluated?
- Which MCF register allocation paths were taken?

When a fuzzing input discovers a *new* path through the compiler (e.g., it triggers a rare macro expansion rule), that input is added to the **Central Corpus**. The fuzzer then uses this input as a seed to mutate further, deliberately trying to push deeper into unexplored compiler logic.

**The Content-Addressable Corpus:**
All test cases, crash dumps, and minimized reproduction cases are stored in a distributed Content-Addressable Storage (CAS) system, identical to the compiler's build cache. 
- Thousands of worker nodes pull seeds from the central corpus, mutate them, and run them through the compiler.
- If a worker finds a new coverage path or a crash, it pushes the result back to the central CAS.
- The central server continuously merges the coverage data, ensuring the entire cluster is collaboratively exploring the vast space of possible Rust programs.

### 6. The "Nightly" Gauntlet (Continuous Validation)

Every single commit to the compiler's source code triggers the Gauntlet.

1. **The Quick Suite:** Runs the 10,000 minimized regression test cases. Must pass in under 2 minutes.
2. **The Differential Suite:** Compiles and executes the top 5,000 most complex programs from the corpus against `rustc`. Must match perfectly.
3. **The Invariant Suite:** Compiles the compiler's own source code (Stage 1) with the new changes, then compiles it again (Stage 2). The resulting binaries must be bit-for-bit identical, and the IR verifiers must pass on every single function.
4. **The Fuzzing Swarm:** Dedicates a cluster of 1,000 CPU cores to run the grammar-aware fuzzer for 24 hours. If a single crash is found, the CI fails, and the minimized test case is automatically attached to the commit that introduced the regression.

This is not just testing. This is an adversarial, continuous, mathematical proof of correctness. The compiler is forged in a crucible of millions of failures, ensuring that when it finally ships, it is unbreakable.


# JIT Runtime & Adaptive Compilation: Complete Technical Design

## 1. Region-Based Partial Deoptimization on Sea of Nodes

### 1.1 The Core Paradigm: Granular Deoptimization
Traditional deoptimization discards the entire function's optimized state and restarts from the beginning. This is catastrophic for performance when only a small region (e.g., a single loop or branch) has violated its assumptions.

**Region-Based Partial Deoptimization** operates on the Sea of Nodes by treating the IR as a collection of **independent, optimizable regions** separated by **deoptimization boundaries**.

**Mathematical Model:**
Let $N$ be the set of nodes in the Sea of Nodes. A **Region** $R_i \subseteq N$ is a connected subgraph with the following properties:
- **Entry Points:** A set of nodes $E(R_i)$ where control can enter the region
- **Exit Points:** A set of nodes $X(R_i)$ where control leaves the region
- **Guards:** A set of guard nodes $G(R_i) \subseteq R_i$ that check runtime assumptions
- **Deoptimization Map:** A function $\delta: G(R_i) \rightarrow \text{State}$ that maps each guard to the unoptimized state it should restore

When a guard $g \in G(R_i)$ fails, only region $R_i$ deoptimizes. The rest of the function continues executing in its optimized form.

### 1.2 Region Construction Algorithm
**Phase 1: Guard Placement**
During AOT compilation, the compiler identifies "speculative opportunities" (e.g., type-specialized paths, devirtualized calls, eliminated bounds checks). At each opportunity, it inserts a **Guard Node** into the Sea of Nodes:
```
GuardNode {
    condition: Node,           // The runtime check
    deopt_state: DeoptState,   // The unoptimized state to restore
    region_id: RegionId,       // Which region this guard belongs to
    priority: u32              // How likely this guard is to fail
}
```

**Phase 2: Region Partitioning**
The compiler partitions the Sea of Nodes into regions using a **dominator-tree-based algorithm**:
1. Compute the dominator tree of the control flow graph
2. For each guard node $g$, find its **immediate dominator** $idom(g)$
3. The region $R_g$ consists of all nodes dominated by $idom(g)$ until the next guard or function exit
4. Merge small regions (< 10 nodes) to reduce deoptimization overhead
5. Split large regions (> 1000 nodes) to improve deoptimization granularity

**Phase 3: Deoptimization State Extraction**
For each region $R_i$, the compiler computes a **Deoptimization State** $\delta(R_i)$:
- **Live Values:** The set of values that are live at the region's entry points
- **Materialization Points:** For each live value, the exact location (register, stack slot, or constant) in the unoptimized code
- **Reconstruction Instructions:** A sequence of instructions to rebuild the unoptimized state from the optimized state

This state is encoded as a **compact bitstream** (typically < 100 bytes per region) and stored in a **Deoptimization Metadata Table** indexed by region ID.

### 1.3 Partial Deoptimization Execution
When a guard fails at runtime:
1. The guard triggers a **trap** (e.g., `ud2` on x86, `brk` on ARM)
2. The runtime's **Deoptimization Handler** reads the trap's instruction pointer
3. It looks up the region ID and deoptimization state from the metadata table
4. It **partially reconstructs** the unoptimized state:
   - Copies live values from optimized registers/stack to unoptimized locations
   - Materializes any scalar-replaced objects (using PEA metadata)
   - Restores the control flow state (which basic block, which instruction)
5. It **patches the call site** to redirect future executions to the unoptimized path
6. Execution resumes in the unoptimized code **at the exact point of failure**

**Key Insight:** Only the failing region's state is reconstructed. The rest of the function's optimized state remains intact. This reduces deoptimization overhead from $O(N)$ to $O(|R_i|)$ where $|R_i|$ is the size of the failing region.

### 1.4 Integration with Sea of Nodes
The Sea of Nodes IR is extended with:
- **Region Nodes:** Meta-nodes that mark region boundaries
- **Guard Nodes:** Conditional nodes that check assumptions and trigger deoptimization
- **Deoptimization Edges:** Control-flow edges that connect guards to deoptimization stubs
- **Materialization Nodes:** Nodes that reconstruct scalar-replaced objects on deoptimization

The borrow checker's liveness bitsets are reused to compute the deoptimization state, ensuring zero redundant analysis.

---

## 2. Lowest Overhead On-Stack Replacement (OSR)

### 2.1 The OSR Challenge
On-Stack Replacement allows jumping from unoptimized code into the middle of optimized code (e.g., entering a hot loop that has already been JIT-compiled). The challenge is minimizing the cost of this transition, which traditionally involves:
- Stopping the world
- Walking the stack
- Reconstructing the entire function's state
- Resuming execution

### 2.2 The Zero-Copy OSR Design
Our OSR achieves **sub-microsecond** transition time through three key innovations:

**Innovation 1: Pre-Computed OSR Entry Points**
During JIT compilation, the compiler generates **specialized OSR entry points** for each loop header and function entry. These entry points are **not** generic deoptimization stubs; they are **hand-optimized assembly sequences** that:
- Accept the unoptimized state as input (in specific registers)
- Perform minimal state transformation (register moves, stack adjustments)
- Jump directly into the optimized code at the exact instruction

The OSR entry point is generated by running the **DP-on-DAG instruction selector** on a "state transformation DAG" that maps unoptimized locations to optimized locations.

**Innovation 2: Shadow Stack Frames**
The runtime maintains a **shadow stack** that mirrors the call stack but stores **OSR metadata**:
- For each stack frame, the shadow stack stores a pointer to the OSR entry point (if one exists)
- The shadow stack is updated on every function call/return using **instrumented prologues/epilogues**
- The overhead is a single pointer write per call (amortized to < 1 cycle)

When OSR is triggered:
1. The runtime reads the shadow stack to find the OSR entry point
2. It jumps directly to the entry point (no stack walking)
3. The entry point performs the state transformation and resumes execution

**Innovation 3: Lazy State Materialization**
Not all live values need to be materialized immediately. The OSR entry point uses **lazy materialization**:
- Values that are immediately used are materialized eagerly
- Values that are not used for several instructions are materialized lazily (on first use)
- This is implemented by inserting **materialization guards** into the optimized code that check if a value has been materialized

### 2.3 OSR Trigger Mechanism
OSR is triggered by **hotness counters** embedded in the unoptimized code:
- Each loop header has a **back-edge counter** (a 32-bit integer in memory)
- Each function entry has an **invocation counter**
- When a counter exceeds a threshold (e.g., 10,000 for loops, 1,000 for functions), it triggers OSR

The trigger mechanism is **asynchronous**:
1. The counter overflow triggers a **signal handler** (or a **polling check** in the unoptimized code)
2. The signal handler submits a **JIT compilation request** to the background compiler thread
3. Once JIT compilation completes, the runtime **patches the unoptimized code** to jump to the OSR entry point
4. The next time the loop is executed, it enters the optimized code via OSR

### 2.4 OSR State Reconstruction
The state reconstruction is performed by the **OSR Entry Point Generator**, which:
1. Takes the unoptimized stack frame layout and the optimized stack frame layout as input
2. Computes a **minimal set of moves** to transform the unoptimized state into the optimized state
3. Uses the **MCF register allocator** to assign physical registers to the moves
4. Emits the moves as a sequence of machine instructions

The generated OSR entry point is **tail-call optimized** to jump directly into the optimized code without returning to the caller.

### 2.5 OSR Performance Characteristics
- **Trigger overhead:** < 1 cycle (counter increment)
- **OSR transition time:** 100-500 nanoseconds (depending on state complexity)
- **Memory overhead:** 8 bytes per stack frame (shadow stack pointer)
- **Code size overhead:** ~50 bytes per OSR entry point

---

## 3. The Runtime Code Emitter

### 3.1 Architecture Overview
The Runtime Code Emitter is the JIT backend that generates machine code directly into executable memory. It operates on the **LIR (Low-level IR)** produced by the middle-end and emits **Machine IR** that is immediately executable.

**Key Components:**
1. **Executable Memory Manager:** Allocates and manages W^X-compliant memory pages
2. **Instruction Encoder:** Converts Machine IR into binary machine code
3. **Relocation Resolver:** Patches addresses and offsets into the emitted code
4. **Cache Coherency Manager:** Ensures the instruction cache is synchronized with the data cache
5. **Code Patching Engine:** Allows runtime modification of emitted code (for deoptimization, OSR, etc.)

### 3.2 Executable Memory Manager
**W^X Compliance:**
Modern OSes enforce that memory pages cannot be both writable and executable simultaneously. The memory manager uses a **three-phase allocation protocol**:
1. **Allocate:** Request memory with `PROT_READ | PROT_WRITE` (writable, not executable)
2. **Emit:** Write machine code into the memory
3. **Finalize:** Change permissions to `PROT_READ | PROT_EXEC` (executable, not writable)

**Memory Layout:**
The emitter uses a **bump allocator** within each memory page:
- Each page is 4KB (or 2MB for large pages)
- Code is emitted sequentially, advancing a pointer
- When a page is full, a new page is allocated
- Pages are linked in a **code segment list** for efficient deallocation

**Memory Reclamation:**
When JIT-compiled code is no longer needed (e.g., after deoptimization), the memory manager:
1. Marks the code as "cold"
2. After a grace period (e.g., 10 seconds), reclaims the memory
3. Uses **reference counting** to ensure no live pointers exist

### 3.3 Instruction Encoder
The instruction encoder converts Machine IR into binary machine code. It is **target-specific** and implements the encoding rules for each ISA (x86-64, ARM64, RISC-V, etc.).

**Encoding Process:**
1. **Instruction Selection:** The DP-on-DAG selector has already chosen the optimal instruction sequence
2. **Operand Encoding:** For each instruction, the encoder:
   - Encodes the opcode (e.g., `0x01` for `add` on x86)
   - Encodes the register operands (e.g., `rax` = 0, `rbx` = 3)
   - Encodes immediate values (e.g., `42` = `0x2a`)
   - Encodes memory operands (e.g., `[rbx + rcx*4 + 16]`)
3. **Relocation Generation:** For external references (e.g., calls to other functions), the encoder generates **relocation records** that will be resolved later

**Max-Perf Encoding:**
The encoder uses **pre-computed encoding tables** to avoid branching:
- Each opcode has a fixed encoding template
- Operand types are encoded using lookup tables
- The encoder processes instructions in batches of 16 for SIMD optimization

### 3.4 Relocation Resolver
Relocations are patches that must be applied to the emitted code when final addresses are known.

**Relocation Types:**
- **Internal Relocations:** References within the same JIT-compiled function (e.g., loop back-edges)
- **External Relocations:** References to AOT-compiled code or runtime functions
- **Data Relocations:** References to global variables or constants

**Resolution Process:**
1. **Internal Relocations:** Resolved immediately during emission (the target address is known)
2. **External Relocations:** Resolved lazily on first use (using a **Global Offset Table** or **Procedure Linkage Table**)
3. **Data Relocations:** Resolved during finalization (when the data section is allocated)

### 3.5 Cache Coherency Manager
After emitting code, the instruction cache (icache) must be synchronized with the data cache (dcache).

**Coherency Protocol:**
1. **Flush Dcache:** Ensure all written data is visible to the icache
2. **Invalidate Icache:** Clear any stale instructions from the icache
3. **Barrier:** Insert a memory barrier to ensure ordering

**Target-Specific Implementation:**
- **x86-64:** The icache is coherent with the dcache, so no explicit flush is needed (but a `mfence` is recommended)
- **ARM64:** Requires explicit `dc cvau` (data cache clean) and `ic ivau` (instruction cache invalidate) instructions
- **RISC-V:** Requires `fence.i` instruction

### 3.6 Code Patching Engine
The code patching engine allows runtime modification of emitted code. This is used for:
- **Deoptimization:** Patching a call site to redirect to a deoptimization stub
- **OSR:** Patching a loop header to redirect to an OSR entry point
- **Inline Cache Updates:** Patching a call site to redirect to a specialized version

**Patching Protocol:**
1. **Suspend Execution:** Ensure no thread is executing the code being patched (using **safepoints** or **thread suspension**)
2. **Make Writable:** Temporarily change the memory page to writable
3. **Patch:** Write the new instructions
4. **Flush Cache:** Invalidate the icache for the patched region
5. **Make Executable:** Restore the memory page to executable
6. **Resume Execution:** Allow threads to continue

**Atomic Patching:**
For small patches (≤ 8 bytes), the emitter uses **atomic writes** to ensure that no thread sees a partially-patched instruction. For larger patches, it uses **thread suspension** to ensure safety.

---

## 4. AOT to JIT Transition via Incremental Compilation

### 4.1 The Core Insight: Don't Recompile, Retune
The key innovation is that the JIT does **not** recompile from scratch. Instead, it uses the existing AOT infrastructure (Autotuner, Sea of Nodes, PEA) and applies **incremental changes** based on runtime profile data.

**The Transition Pipeline:**
1. **AOT Compilation:** The compiler generates optimized AOT code using the Autotuner's ML-guided search
2. **Runtime Profiling:** The AOT code collects profile data (type frequencies, branch outcomes, call targets)
3. **Profile-Driven Retuning:** When the profile data diverges significantly from the AOT assumptions, the JIT:
   - Loads the cached AOT IR from the CAS
   - Applies **incremental transformations** based on the new profile data
   - Re-runs the Autotuner with the updated profile as input
   - Emits the updated code via the Runtime Code Emitter
4. **OSR Transition:** The runtime transitions to the new code via OSR

### 4.2 Incremental Autotuner Updates
The Autotuner's 4-tier funnel is reused, but with **incremental evaluation**:

**Tier 0 (Surrogate Inference):**
- The CGT (Conditional Graph Transformer) is re-run with the updated profile data
- It predicts which passes will be most beneficial given the new profile
- The bottom 90% of candidates are discarded

**Tier 1 (Partial IR Execution):**
- Only the **affected regions** are re-evaluated (not the entire function)
- The actual IR delta is compared to the predicted delta
- If the deviation is too large, the candidate is discarded

**Tier 2 (Cycle-Accurate Simulation):**
- The simulator re-runs only the affected regions
- It uses the existing simulation results as a baseline and applies deltas

**Tier 3 (Silicon Validation):**
- Only the top contenders are validated on silicon
- The validation is **incremental**: it measures the performance delta, not the absolute performance

### 4.3 CAS-Based Incremental Compilation
The Content-Addressable Storage (CAS) is extended to cache **profile-specific configurations**:

**Cache Key:**
```
CompositeHash {
    function_lir_hash: Hash,
    target_hardware_fingerprint: Hash,
    profile_hash: Hash,           // NEW: Hash of the runtime profile
    compiler_version: Hash
}
```

**Cache Value:**
```
TunedConfiguration {
    plackett_luce_logits: Vec<f32>,
    iteration_counts: Vec<u32>,
    factor_values: Vec<f32>,
    deoptimization_metadata: DeoptMetadata,
    osr_entry_points: Vec<OSREntryPoint>
}
```

**Cache Lookup:**
When a function needs to be JIT-compiled:
1. Compute the composite hash (including the current profile)
2. Look up the hash in the CAS
3. If found, load the cached configuration and emit the code immediately
4. If not found, run the incremental Autotuner and cache the result

### 4.4 Zero-Waste Transition
The incremental compilation ensures that **nothing is wasted**:
- The AOT code's optimization results are reused as a baseline
- The Autotuner's ML model is fine-tuned, not retrained
- The CAS cache is extended, not replaced
- The runtime profile data is accumulated, not discarded

This results in **sub-second** JIT compilation times for most functions, as the heavy lifting (Autotuner search, IR analysis) has already been done during AOT compilation.

---

## 5. JIT-Specific Optimizations: Complete Catalog

### 5.1 Inline Caching
**Monomorphic Inline Cache (MIC):**
- Caches a single type at a call site
- If the type matches, execution continues without a call
- If the type mismatches, transitions to polymorphic or megamorphic

**Polymorphic Inline Cache (PIC):**
- Caches up to 4 types at a call site
- Uses a linear search or binary search to find the matching type
- If the type is not found, transitions to megamorphic

**Megamorphic Dispatch:**
- Falls back to a global hash table lookup
- Or generates a specialized dispatch stub via JIT

**Implementation:**
The inline cache is embedded directly in the machine code as a sequence of compare-and-branch instructions. The cache is updated by **patching the code** when a new type is encountered.

### 5.2 Speculative Optimization
**Type Specialization:**
- The JIT speculates that a variable has a specific type (e.g., `i32`)
- It generates optimized code for that type
- It inserts a guard to check the type at runtime
- If the guard fails, it deoptimizes to the generic code

**Devirtualization:**
- The JIT speculates that a virtual call resolves to a specific method
- It inlines the method directly
- It inserts a guard to check the method's identity
- If the guard fails, it deoptimizes to the virtual call

**Bounds Check Elimination:**
- The JIT speculates that an array index is within bounds
- It removes the bounds check
- It inserts a guard to check the bounds at runtime
- If the guard fails, it deoptimizes to the checked code

### 5.3 Guarded Optimizations
**Guard Insertion:**
- For each speculative optimization, the JIT inserts a guard node
- The guard checks the assumption (e.g., type, bounds, method identity)
- If the guard fails, it triggers deoptimization

**Guard Elision:**
- If the profile data shows that a guard always succeeds, the JIT may elide the guard
- This is done by the Autotuner, which prioritizes guard elision for hot paths

**Guard Coalescing:**
- Multiple guards checking the same assumption are coalesced into a single guard
- This reduces the overhead of guard checking

### 5.4 Runtime Constant Propagation
**Profile-Guided Constants:**
- If the profile data shows that a variable is always a specific constant, the JIT treats it as a constant
- It propagates the constant through the IR
- It eliminates redundant computations

**Lazy Constant Folding:**
- Constants are folded lazily, only when they are used
- This avoids folding constants that are never used

### 5.5 Profile-Guided Inlining
**Hot Call Site Inlining:**
- The JIT inlines functions that are called frequently
- It uses the profile data to determine which call sites are hot

**Cold Call Site Outlining:**
- The JIT outlines functions that are called infrequently
- This reduces code size and improves icache locality

**Speculative Inlining:**
- The JIT speculatively inlines a function based on the call site's type profile
- If the type profile changes, it deoptimizes and re-inlines

### 5.6 Dynamic Class Hierarchy Analysis (CHA)
**Class Hierarchy Tracking:**
- The runtime tracks the class hierarchy dynamically
- When a new class is loaded, it updates the hierarchy

**Devirtualization via CHA:**
- If CHA shows that a virtual method has only one implementation, the JIT devirtualizes the call
- If a new implementation is loaded, the JIT deoptimizes and re-compiles

### 5.7 Runtime Loop Optimizations
**Profile-Guided Loop Unrolling:**
- The JIT unrolls loops based on the profile data (e.g., trip count, iteration variability)
- It uses the Autotuner to determine the optimal unroll factor

**Runtime Loop Tiling:**
- The JIT tiles loops based on the profile data (e.g., cache miss rates, memory access patterns)
- It uses the Autotuner to determine the optimal tile size

**Adaptive Loop Vectorization:**
- The JIT vectorizes loops based on the profile data (e.g., data alignment, stride)
- It uses the Autotuner to determine the optimal vector width

### 5.8 Dynamic Register Allocation
**Profile-Guided Register Allocation:**
- The MCF register allocator uses profile data to determine which values are most frequently used
- It assigns registers to the most frequently used values
- It spills the least frequently used values

**Runtime Register Pressure Relief:**
- If the register pressure is too high, the JIT dynamically re-allocates registers
- It uses the MCF solver to find a better allocation

### 5.9 Runtime Instruction Selection
**Profile-Guided Instruction Selection:**
- The DP-on-DAG instruction selector uses profile data to determine which instruction patterns are most beneficial
- It selects patterns that minimize latency for the actual data patterns

**Adaptive Addressing Mode Selection:**
- The JIT selects addressing modes based on the profile data (e.g., stride, alignment)
- It uses the Autotuner to determine the optimal addressing mode

### 5.10 Lazy Compilation
**Compile-on-First-Use:**
- Functions are compiled lazily, only when they are first called
- This reduces startup time and memory usage

**Tiered Compilation:**
- **Tier 0:** Interpreter (no compilation, just execution)
- **Tier 1:** Quick JIT (minimal optimization, fast compilation)
- **Tier 2:** Optimized JIT (full optimization, slower compilation)
- Functions progress through the tiers based on their hotness

### 5.11 Runtime Memory Optimizations
**Profile-Guided Allocation:**
- The JIT uses profile data to determine which allocations can be scalar-replaced
- It uses PEA to eliminate allocations on the hot path

**Runtime Garbage Collection Hints:**
- The JIT inserts GC safepoints at optimal locations based on the profile data
- It minimizes the overhead of GC pauses

### 5.12 Adaptive Branch Prediction
**Profile-Guided Branch Hints:**
- The JIT inserts branch prediction hints based on the profile data
- It uses the `likely` and `unlikely` attributes to guide the CPU's branch predictor

**Runtime Branch Reordering:**
- The JIT reorders branches based on the profile data
- It places the most likely branch first to improve branch prediction

### 5.13 Dynamic Dispatch Optimization
**Call Site Specialization:**
- The JIT specializes call sites based on the type profile
- It generates specialized code for each type

**Megamorphic Dispatch Stubs:**
- For highly dynamic call sites, the JIT generates specialized dispatch stubs
- These stubs use hash tables or binary search to find the target method

### 5.14 Runtime Exception Handling
**Lazy Unwind Table Generation:**
- The JIT generates unwind tables lazily, only when an exception is thrown
- This reduces the overhead of exception handling in the common case

**Optimized Landing Pads:**
- The JIT generates optimized landing pads based on the profile data
- It minimizes the overhead of exception handling

### 5.15 Cross-Function Optimization
**Runtime Inlining Across Module Boundaries:**
- The JIT inlines functions across module boundaries based on the profile data
- It uses LTO (Link-Time Optimization) results as a baseline

**Runtime Devirtualization Across Module Boundaries:**
- The JIT devirtualizes calls across module boundaries based on the profile data
- It uses CHA (Class Hierarchy Analysis) to determine the target method

### 5.16 Self-Optimizing Code
**Runtime Code Rewriting:**
- The JIT rewrites its own code based on the profile data
- It uses the Runtime Code Emitter to patch the code in place

**Adaptive Code Layout:**
- The JIT reorders code based on the profile data
- It places hot code at the beginning of the function to improve icache locality

**Runtime Dead Code Elimination:**
- The JIT eliminates dead code based on the profile data
- It removes code that is never executed

---

## Summary

This design transforms the compiler from a pure AOT system into a **hybrid AOT/JIT system** that leverages the existing infrastructure (Autotuner, Sea of Nodes, PEA, CAS) to achieve:

1. **Region-Based Partial Deoptimization:** Granular deoptimization that only affects the failing region, not the entire function
2. **Lowest Overhead OSR:** Sub-microsecond transitions using pre-computed entry points, shadow stacks, and lazy materialization
3. **Runtime Code Emitter:** A high-performance JIT backend that emits machine code directly into executable memory with W^X compliance
4. **Incremental AOT→JIT Transition:** Zero-waste transitions that reuse AOT results and apply only the necessary changes
5. **Complete JIT Optimization Catalog:** 16 categories of JIT-specific optimizations that leverage runtime profile data

