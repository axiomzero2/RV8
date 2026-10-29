// CEP:FILE: crates/rv8_autotuner/src/lib.rs
// CEP:WHAT: rv8_autotuner, the ML autotuner of the RV8 compiler: Plackett-Luce pass-ordering
//   genomes, one conditional graph transformer surrogate with delta and value heads and MC-dropout
//   uncertainty, multi-fidelity acquisition divided by fidelity cost, the four-tier evaluation
//   funnel, MinHash/Jaccard warm start, and a content-addressed configuration cache whose hits
//   replay prior configurations bit-for-bit.
// CEP:WHY: Pass-ordering decisions at RV8's scale cannot be hand-tuned; standard 5.3 fixes the
//   architecture (one surrogate, one acquisition rule, one funnel) and this crate implements it
//   with zero external dependencies, total determinism under content-derived seeds, and no panic
//   paths reachable from library code.
// CEP:CLASS: CEP-1
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-132
// CEP:WHY-PARTIAL: every module is complete against its stated contract except the CGT
//   surrogate, which is partial by design (real architecture, deterministic seeded init, online
//   readout-only SGD at surrogate scale; see cgt.rs CEP:WHY-PARTIAL) and the acquisition rule,
//   which is an honest MF-PES proxy rather than computed information gain (see acquisition.rs
//   CEP:WHY-PARTIAL). Tier execution (partial-IR runs, cycle simulation, silicon measurement) is
//   outside this crate: it plugs in through budgets and measured observations supplied by the
//   driver.
// CEP:FAILURE: no public function panics; every fallible operation returns Result<_, AutotunerError>
//   or Option; non-finite floats are rejected at validated boundaries so NaN never propagates.
// CEP:ASSUMES: host provides std; callers derive LIR hashes and hardware fingerprints with
//   run-stable hashing (rv8_core fxhash64/SHA-256).
// CEP:COST: tuning is offline work; the only hot-path operation is Tuner::lookup, a zero-allocation
//   BTreeMap::get.
// CEP:EVIDENCE: unit tests across all eight modules (see each file's CEP:EVIDENCE); acceptance
//   evidence in tuner::tests::cache_replay_bit_for_bit (bit-for-bit replay) and
//   tuner::tests::lookup_large_cache_returns_correct_entry (10k-entry lookup path).

pub mod acquisition;
pub mod cgt;
pub mod error;
pub mod funnel;
pub mod genome;
pub mod minhash;
pub mod pl;
pub mod tuner;

pub use acquisition::{acquire, CONFIDENCE_BONUS_SCALE};
pub use cgt::{
    Cgt, Prediction, CGT_INIT_SEED, DROPOUT_PASS_MIX, DROPOUT_RATE, DROPOUT_SEED, EDGE_KIND_SLOTS,
    EMBED_DIM, FEATURE_DIM, GENOME_HASH_SLOTS, GENOME_RAW_DIM, GRAD_NORM_FLOOR, HEAD_DIM, HEADS,
    INIT_BOUND, LEARNING_RATE, LOOP_DEPTH_SLOTS, MC_DROPOUT_PASSES, NODE_CLASS_SLOTS,
    REG_PRESSURE_SLOTS, TOKEN_COUNT, TOKEN_DIM,
};
pub use error::AutotunerError;
pub use funnel::{
    Funnel, Tier, MIN_KEEP_COUNT, SILICON_KEEP_FRACTION, TIER0_KEEP_FRACTION,
    TIER0_SURROGATE_COST, TIER1_KEEP_FRACTION, TIER1_PARTIAL_IR_COST, TIER2_CYCLE_SIM_COST,
    TIER2_KEEP_FRACTION, TIER3_SILICON_COST,
};
pub use genome::{
    Genome, CROSSOVER_OTHER_PROB, DISTANCE_FACTOR_WEIGHT, DISTANCE_ITERATION_WEIGHT,
    DISTANCE_LOGIT_WEIGHT, FACTOR_MAX, FACTOR_MIN, GENOME_HASH_SALT, ITERATIONS_MAX, ITERATIONS_MIN,
    MAX_PASS_COUNT, MUTATION_FACTOR_SCALE, MUTATION_ITERATION_MAX, MUTATION_LOGIT_SCALE,
};
pub use minhash::{
    jaccard, minhash_signature, warm_start, EMPTY_SET_MINIMUM, MINHASH_ITEM_SALT, WARM_START_BLEND,
    WARM_START_K, WARM_START_SEED,
};
pub use pl::{order_log_probability, sample_order};
pub use tuner::{tuner_seed, CachedConfig, Tuner, TunerKey, LOCAL_MUTATION_ROLLOUTS, TUNER_SEED_ROTATE_BITS};
