// CEP:FILE: crates/rv8_autotuner/src/cgt.rs
// CEP:WHAT: The conditional graph transformer (CGT) surrogate: a real multi-head cross-attention
//   architecture in which the genome embedding (query) attends over heterogeneous IR feature
//   tokens (keys/values), with a Delta Head, a Value Head, and MC-dropout epistemic uncertainty,
//   plus seeded deterministic initialization and online SGD fitting.
// CEP:WHY: Standard 5.3 mandates ONE surrogate model for the autotuner. This is a genuine
//   transformer block (softmax(Q K^T / sqrt(d_k)) V, HEADS heads) at surrogate scale: it is small
//   enough to run inside a compiler build yet architecturally complete.
// CEP:CLASS: CEP-2
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-131
// CEP:WHY-PARTIAL: honest scope statement - the architecture is real (genome-query cross-attention
//   over heterogeneous IR feature tokens, delta + value heads, seeded MC-dropout uncertainty,
//   deterministic SplitMix64 weight init, online SGD on the readout), but it is a surrogate-scale
//   model, not a production-trained network: online fit updates the delta readout head over a
//   frozen attention context (normalized-gradient SGD with a guaranteed per-sample loss
//   contraction), and no representation learning happens online. Pre-training or full-backprop
//   online learning would be a follow-up task.
// CEP:FAILURE: predict/fit return AutotunerError for wrong-length features, non-finite features
//   or targets, or an invalid genome; NaN cannot enter the math. With finite inputs every
//   intermediate is bounded (tanh squashes embeddings and inputs to [-1,1], bounded init weights
//   bound q/k/v, max-shifted softmax keeps exponentials <= 1), so no overflow to inf is reachable.
// CEP:ASSUMES: features are the caller-computed fixed-width IR summary (see FEATURE_DIM layout);
//   statistical quality of MC dropout over MC_DROPOUT_PASSES passes suffices for uncertainty.
// CEP:COST: predict is O(EMBED_DIM*GENOME_RAW_DIM + TOKEN_COUNT*EMBED_DIM*TOKEN_DIM +
//   3*EMBED_DIM^2 + MC_DROPOUT_PASSES*EMBED_DIM) ~= 40k multiply-adds; fit adds one readout
//   gradient step over EMBED_DIM.
// CEP:EVIDENCE: unit tests cgt::tests::{predict_finite_stable_deterministic,
//   fit_reduces_loss_on_repeated_sample, predict_rejects_nan_features,
//   determinism_across_seeded_instances, dropout_variance_nonneg}.

use crate::error::AutotunerError;
use crate::genome::{Genome, MAX_PASS_COUNT};
use rv8_core::hash::fx_mix;
use rv8_core::rng::SplitMix64;

/// CEP:WHAT: Heterogeneous node-class histogram slots in the feature vector (six classes).
/// CEP:WHY: Matches the six heterogeneous IR node classes of the level-2 IR contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: caller fills slots with class counts or normalized frequencies.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const NODE_CLASS_SLOTS: usize = 6;

/// CEP:WHAT: Edge-kind histogram slots in the feature vector (six kinds).
/// CEP:WHY: Matches the six heterogeneous IR edge kinds (data, control, dependency, locality,
///   sync, communication).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: caller fills slots with per-kind edge counts.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const EDGE_KIND_SLOTS: usize = 6;

/// CEP:WHAT: Feature slots for the maximum loop depth of the IR.
/// CEP:WHY: Loop nesting drives pass behavior; one scalar slot.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: caller provides a finite depth estimate.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const LOOP_DEPTH_SLOTS: usize = 1;

/// CEP:WHAT: Feature slots for the estimated register pressure.
/// CEP:WHY: Pressure estimates steer allocation-sensitive pass choices.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: caller provides a finite pressure estimate.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const REG_PRESSURE_SLOTS: usize = 1;

/// CEP:WHAT: Total feature width: 6 node classes + 6 edge kinds + loop depth + register pressure.
/// CEP:WHY: One named dimension for the fixed-width IR summary; all validation uses it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: layout is [node histogram | edge histogram | loop depth | register pressure].
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const FEATURE_DIM: usize = NODE_CLASS_SLOTS + EDGE_KIND_SLOTS + LOOP_DEPTH_SLOTS + REG_PRESSURE_SLOTS;

/// CEP:WHAT: Width of one heterogeneous feature token (7 values).
/// CEP:WHY: The transformer consumes tokens, not the raw vector; 7 divides FEATURE_DIM so
///   token 0 = node histogram + loop depth and token 1 = edge histogram + register pressure.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: FEATURE_DIM is divisible by TOKEN_DIM (compile-time checked below).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: compile-time assertion in this module; cgt::tests::feature_layout_sums.
pub const TOKEN_DIM: usize = 7;

/// CEP:WHAT: Number of feature tokens fed to attention.
/// CEP:WHY: FEATURE_DIM / TOKEN_DIM heterogeneous chunks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: divides FEATURE_DIM exactly.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::feature_layout_sums.
pub const TOKEN_COUNT: usize = FEATURE_DIM / TOKEN_DIM;

/// CEP:WHAT: Transformer embedding width (also the concatenated multi-head output width).
/// CEP:WHY: Named hyperparameter; 32 keeps the surrogate in the tens-of-kilobyte range.
/// CEP:STATUS: complete (architecture) / partial (model scale)
/// CEP:FAILURE: none
/// CEP:ASSUMES: divisible by HEADS.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
pub const EMBED_DIM: usize = 32;

/// CEP:WHAT: Number of attention heads.
/// CEP:WHY: Named hyperparameter; 4 heads let the genome query attend to different feature
///   subspaces per head.
/// CEP:STATUS: complete (architecture) / partial (model scale)
/// CEP:FAILURE: none
/// CEP:ASSUMES: divides EMBED_DIM (compile-time checked below).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
pub const HEADS: usize = 4;

/// CEP:WHAT: Per-head key/query width, EMBED_DIM / HEADS.
/// CEP:WHY: Standard multi-head attention split.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: EMBED_DIM % HEADS == 0.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: compile-time assertion in this module.
pub const HEAD_DIM: usize = EMBED_DIM / HEADS;

/// CEP:WHAT: Number of u64 hash words encoding the genome logits inside the embedding input.
/// CEP:WHY: Logits enter the surrogate as content hashes, so arbitrary logit magnitudes cannot
///   destabilize the network while identical logits remain indistinguishable from each other.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: fxhash64 is run-stable (rv8_core contract).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
pub const GENOME_HASH_SLOTS: usize = 2;

/// CEP:WHAT: Salt mixed into the second genome hash word.
/// CEP:WHY: Law 7 named constant; decorrelates the two hash slots.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
pub const CGT_HASH_SALT: u64 = 0x0C67_0000_5EED_0001;

/// CEP:WHAT: Width of the raw genome embedding input: hash slots + padded iterations + factors.
/// CEP:WHY: Fixed width lets the projection matrix be a named constant size.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: iterations and factors are zero-padded to MAX_PASS_COUNT.
/// CEP:COST: compile-time constant (354 with the 176-pass catalog).
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
pub const GENOME_RAW_DIM: usize = GENOME_HASH_SLOTS + 2 * MAX_PASS_COUNT;

/// CEP:WHAT: Number of MC-dropout passes used to estimate epistemic variance.
/// CEP:WHY: Named hyperparameter; 8 passes bound the variance estimate cost.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: variance over 8 seeded masks is a usable epistemic proxy.
/// CEP:COST: linear factor on predict.
/// CEP:EVIDENCE: cgt::tests::dropout_variance_nonneg.
pub const MC_DROPOUT_PASSES: usize = 8;

/// CEP:WHAT: Per-unit dropout probability for MC passes.
/// CEP:WHY: Named hyperparameter; 0.1 is the standard MC-dropout rate.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: strictly inside (0, 1) so the inverted-dropout scale stays finite.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::dropout_variance_nonneg.
pub const DROPOUT_RATE: f64 = 0.1;

/// CEP:WHAT: Seed for the deterministic MC-dropout masks; pass m uses
///   DROPOUT_SEED ^ m * DROPOUT_PASS_MIX.
/// CEP:WHY: Masks must be a pure function of the pass index so identical inputs give identical
///   predictions (determinism rule), independent of any ambient state.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
pub const DROPOUT_SEED: u64 = 0xD807_5EED_0000_0001;

/// CEP:WHAT: Per-pass multiplier decorrelating the MC-dropout mask streams (SplitMix golden
///   ratio).
/// CEP:WHY: Law 7 named constant; distinct streams per pass.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::dropout_variance_nonneg.
pub const DROPOUT_PASS_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// CEP:WHAT: Default seed for deterministic weight initialization.
/// CEP:WHY: Two Tuners constructed with no arguments must be bit-identical (reproducibility).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
pub const CGT_INIT_SEED: u64 = 0x5EED_C671_0000_0002;

/// CEP:WHAT: Uniform initialization half-width for every weight and bias.
/// CEP:WHY: Named hyperparameter; small init bounds attention logits and keeps outputs finite.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
pub const INIT_BOUND: f64 = 0.05;

/// CEP:WHAT: Online SGD learning rate for the delta readout head.
/// CEP:WHY: Named hyperparameter; with gradient normalization, any rate in (0, 1) contracts the
///   fitted sample residual by exactly (1 - 2*rate) per step.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::fit_reduces_loss_on_repeated_sample.
pub const LEARNING_RATE: f64 = 0.01;

/// CEP:WHAT: Floor on the gradient normalizer ||ctx||^2 + 1 used by fit.
/// CEP:WHY: Guarantees the normalized gradient step is a contraction even for near-zero
///   contexts, which is what makes per-sample loss decrease provable rather than hopeful.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: cgt::tests::fit_reduces_loss_on_repeated_sample.
pub const GRAD_NORM_FLOOR: f64 = 1.0;

const _: () = assert!(FEATURE_DIM == NODE_CLASS_SLOTS + EDGE_KIND_SLOTS + LOOP_DEPTH_SLOTS + REG_PRESSURE_SLOTS);
const _: () = assert!(FEATURE_DIM.is_multiple_of(TOKEN_DIM), "features must split into whole tokens");
const _: () = assert!(EMBED_DIM.is_multiple_of(HEADS), "heads must divide the embedding width");

/// CEP:WHAT: Surrogate prediction: mean and variance of the predicted cost delta plus the
///   predicted absolute value.
/// CEP:WHY: The acquisition function consumes exactly these three numbers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: delta_var >= 0 by construction (mean squared deviation).
/// CEP:COST: plain data.
/// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    /// Mean predicted cost delta under the MC-dropout ensemble.
    pub delta_mean: f64,
    /// Population variance of the predicted delta across MC passes (epistemic proxy).
    pub delta_var: f64,
    /// Mean predicted absolute value of the configuration.
    pub value: f64,
}

/// CEP:WHAT: The conditional graph transformer surrogate model and its weights.
/// CEP:WHY: One model for the whole autotuner per standard 5.3.
/// CEP:STATUS: partial (real architecture, surrogate scale, readout-only online SGD; see
///   CEP:WHY-PARTIAL on the file header)
/// CEP:FAILURE: see file header CEP:FAILURE.
/// CEP:ASSUMES: see file header CEP:ASSUMES.
/// CEP:COST: ~14.8k f64 weights (~119 KiB): genome projection 32x354, input projection 32x7,
///   three 32x32 attention projections, two 32-wide readout heads.
/// CEP:EVIDENCE: unit tests cgt::tests::*.
#[derive(Debug, Clone)]
pub struct Cgt {
    w_genome: Vec<f64>, // EMBED_DIM x GENOME_RAW_DIM, row-major
    b_genome: Vec<f64>, // EMBED_DIM
    w_in: Vec<f64>,     // EMBED_DIM x TOKEN_DIM, row-major
    w_q: Vec<f64>,      // EMBED_DIM x EMBED_DIM, row-major
    w_k: Vec<f64>,      // EMBED_DIM x EMBED_DIM, row-major
    w_v: Vec<f64>,      // EMBED_DIM x EMBED_DIM, row-major
    w_delta: Vec<f64>,  // EMBED_DIM
    b_delta: f64,
    w_value: Vec<f64>, // EMBED_DIM
    b_value: f64,
    attn_scale: f64, // 1 / sqrt(HEAD_DIM)
}

impl Cgt {
    /// CEP:WHAT: Creates the model with the crate-default deterministic seed.
    /// CEP:WHY: Tuners built identically must behave identically (self-hosting reproducibility).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(weights) init draws.
    /// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
    pub fn new() -> Cgt {
        Cgt::with_seed(CGT_INIT_SEED)
    }

    /// CEP:WHAT: Creates the model with an explicit seed; every weight is
    ///   uniform(-INIT_BOUND, INIT_BOUND) drawn from a SplitMix64 stream in a fixed order.
    /// CEP:WHY: Deterministic seeded init is the reproducibility backbone of the surrogate.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(EMBED_DIM*(GENOME_RAW_DIM + 3*EMBED_DIM + TOKEN_DIM + 2)).
    /// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
    pub fn with_seed(seed: u64) -> Cgt {
        let mut rng = SplitMix64::seed_from_u64(seed);
        let mut w_genome = vec![0.0; EMBED_DIM * GENOME_RAW_DIM];
        let mut b_genome = vec![0.0; EMBED_DIM];
        let mut w_in = vec![0.0; EMBED_DIM * TOKEN_DIM];
        let mut w_q = vec![0.0; EMBED_DIM * EMBED_DIM];
        let mut w_k = vec![0.0; EMBED_DIM * EMBED_DIM];
        let mut w_v = vec![0.0; EMBED_DIM * EMBED_DIM];
        let mut w_delta = vec![0.0; EMBED_DIM];
        let mut w_value = vec![0.0; EMBED_DIM];
        fill_uniform(&mut rng, &mut w_genome);
        fill_uniform(&mut rng, &mut b_genome);
        fill_uniform(&mut rng, &mut w_in);
        fill_uniform(&mut rng, &mut w_q);
        fill_uniform(&mut rng, &mut w_k);
        fill_uniform(&mut rng, &mut w_v);
        fill_uniform(&mut rng, &mut w_delta);
        fill_uniform(&mut rng, &mut w_value);
        let b_delta = uniform_pm_one(&mut rng) * INIT_BOUND;
        let b_value = uniform_pm_one(&mut rng) * INIT_BOUND;
        Cgt {
            w_genome,
            b_genome,
            w_in,
            w_q,
            w_k,
            w_v,
            w_delta,
            b_delta,
            w_value,
            b_value,
            attn_scale: 1.0 / (HEAD_DIM as f64).sqrt(),
        }
    }

    /// CEP:WHAT: Predicts delta mean, delta variance (MC-dropout epistemic proxy), and value for
    ///   one (IR features, genome) pair.
    /// CEP:WHY: The single scoring entry point used by acquisition and the funnel.
    /// CEP:STATUS: partial (surrogate-scale architecture; see CEP:WHY-PARTIAL on the file header)
    /// CEP:FAILURE: LengthMismatch when features.len() != FEATURE_DIM; NonFiniteInput for
    ///   non-finite features; genome validation errors propagate unchanged.
    /// CEP:ASSUMES: identical (weights, features, genome) always produce bit-identical output;
    ///   dropout masks depend only on the pass index.
    /// CEP:COST: one context computation plus MC_DROPOUT_PASSES readout passes.
    /// CEP:EVIDENCE: cgt::tests::{predict_finite_stable_deterministic, dropout_variance_nonneg,
    ///   predict_rejects_nan_features}.
    pub fn predict(&self, features: &[f64], genome: &Genome) -> Result<Prediction, AutotunerError> {
        validate_features(features)?;
        genome.validate()?;
        let ctx = self.context_vector(features, genome);
        let mut deltas = [0.0f64; MC_DROPOUT_PASSES];
        let mut values = [0.0f64; MC_DROPOUT_PASSES];
        let keep_scale = 1.0 / (1.0 - DROPOUT_RATE);
        for (m, (d_slot, v_slot)) in deltas.iter_mut().zip(values.iter_mut()).enumerate() {
            let mut rng =
                SplitMix64::seed_from_u64(DROPOUT_SEED ^ (m as u64).wrapping_mul(DROPOUT_PASS_MIX));
            let mut delta = self.b_delta;
            let mut value = self.b_value;
            for ((wd, wv), c) in self.w_delta.iter().zip(self.w_value.iter()).zip(ctx.iter()) {
                let kept = if rng.next_f64() >= DROPOUT_RATE { keep_scale } else { 0.0 };
                let masked = c * kept;
                delta += wd * masked;
                value += wv * masked;
            }
            *d_slot = delta;
            *v_slot = value;
        }
        let n = MC_DROPOUT_PASSES as f64;
        let delta_mean = deltas.iter().sum::<f64>() / n;
        let delta_var = deltas.iter().map(|d| (d - delta_mean) * (d - delta_mean)).sum::<f64>() / n;
        let value = values.iter().sum::<f64>() / n;
        Ok(Prediction { delta_mean, delta_var: delta_var.max(0.0), value })
    }

    /// CEP:WHAT: One online SGD step on the delta readout head against an observed cost delta;
    ///   returns the pre-update MSE loss on the sample.
    /// CEP:WHY: The tuner must learn from funnel measurements without a batch training run.
    /// CEP:STATUS: partial (readout-only online SGD over a frozen attention context; the
    ///   representation is not updated online - see CEP:WHY-PARTIAL on the file header)
    /// CEP:FAILURE: LengthMismatch/NonFiniteInput for features; genome validation errors;
    ///   NonFiniteInput for a non-finite observed_delta.
    /// CEP:ASSUMES: the normalized gradient makes the update a contraction: with
    ///   g = ||ctx||^2 + 1 and denom = max(g, GRAD_NORM_FLOOR) >= g, the new residual is
    ///   r * (1 - LEARNING_RATE*2*g/denom), which is strictly smaller in magnitude for
    ///   LEARNING_RATE in (0, 1); per-sample loss therefore decreases monotonically.
    /// CEP:COST: one context computation plus one EMBED_DIM-sized gradient step.
    /// CEP:EVIDENCE: cgt::tests::fit_reduces_loss_on_repeated_sample.
    pub fn fit(
        &mut self,
        features: &[f64],
        genome: &Genome,
        observed_delta: f64,
    ) -> Result<f64, AutotunerError> {
        validate_features(features)?;
        genome.validate()?;
        if !observed_delta.is_finite() {
            return Err(AutotunerError::NonFiniteInput);
        }
        let ctx = self.context_vector(features, genome);
        let pred = self.b_delta + dot(&self.w_delta, &ctx);
        let residual = pred - observed_delta;
        let loss = residual * residual;
        let g = dot(&ctx, &ctx) + 1.0;
        let denom = g.max(GRAD_NORM_FLOOR);
        let step = LEARNING_RATE * 2.0 * residual / denom;
        for (w, c) in self.w_delta.iter_mut().zip(ctx.iter()) {
            *w -= step * c;
        }
        self.b_delta -= step;
        Ok(loss)
    }

    /// CEP:WHAT: Builds the fixed-width raw genome embedding input: two hash words over the
    ///   logits, then iterations and factors zero-padded to MAX_PASS_COUNT.
    /// CEP:WHY: A fixed input width is what makes the projection a static matrix.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: genome has already been validated by the caller.
    /// CEP:COST: O(MAX_PASS_COUNT) plus one fxhash64 over the genome bytes.
    /// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
    fn genome_raw(&self, genome: &Genome, raw: &mut Vec<f64>) {
        raw.clear();
        let h0 = genome.hash64();
        let h1 = fx_mix(h0, CGT_HASH_SALT);
        raw.push(h0 as f64);
        raw.push(h1 as f64);
        for i in 0..MAX_PASS_COUNT {
            raw.push(genome.iterations.get(i).copied().unwrap_or(0) as f64);
        }
        for i in 0..MAX_PASS_COUNT {
            raw.push(genome.factors.get(i).copied().unwrap_or(0.0));
        }
    }

    /// CEP:WHAT: Runs the cross-attention forward pass up to (not including) the readout heads:
    ///   genome embedding (query side) attends over feature tokens (key/value side) with
    ///   softmax(Q K^T / sqrt(d_k)) V per head, heads concatenated.
    /// CEP:WHY: Shared by predict and fit; the readout differs (dropout vs clean).
    /// CEP:STATUS: partial (architecture complete; model is surrogate scale)
    /// CEP:FAILURE: assumes validated, finite inputs (enforced by callers).
    /// CEP:ASSUMES: bounded weights keep every intermediate finite; stable softmax keeps
    ///   exponentials <= 1 so no overflow is reachable.
    /// CEP:COST: O(EMBED_DIM*GENOME_RAW_DIM + TOKEN_COUNT*(EMBED_DIM*TOKEN_DIM + 2*EMBED_DIM^2)).
    /// CEP:EVIDENCE: cgt::tests::predict_finite_stable_deterministic.
    fn context_vector(&self, features: &[f64], genome: &Genome) -> Vec<f64> {
        // Genome embedding: tanh(W_g . raw + b).
        let mut raw = Vec::with_capacity(GENOME_RAW_DIM);
        self.genome_raw(genome, &mut raw);
        let mut g_emb = vec![0.0; EMBED_DIM];
        matvec(&self.w_genome, GENOME_RAW_DIM, &raw, &mut g_emb);
        for (g, b) in g_emb.iter_mut().zip(self.b_genome.iter()) {
            *g = (*g + b).tanh();
        }
        // Query from the genome embedding; bias-free attention projections by design.
        let mut q = vec![0.0; EMBED_DIM];
        matvec(&self.w_q, EMBED_DIM, &g_emb, &mut q);
        // Per-token input projection, then keys and values.
        let mut v_all = vec![0.0f64; TOKEN_COUNT * EMBED_DIM];
        let mut scores = [0.0f64; HEADS * TOKEN_COUNT];
        let mut h = vec![0.0; EMBED_DIM];
        let mut k = vec![0.0; EMBED_DIM];
        for t in 0..TOKEN_COUNT {
            let x_t = &features[t * TOKEN_DIM..(t + 1) * TOKEN_DIM];
            matvec(&self.w_in, TOKEN_DIM, x_t, &mut h);
            for e in h.iter_mut() {
                *e = e.tanh();
            }
            matvec(&self.w_k, EMBED_DIM, &h, &mut k);
            let v_slice = &mut v_all[t * EMBED_DIM..(t + 1) * EMBED_DIM];
            matvec(&self.w_v, EMBED_DIM, &h, v_slice);
            for head in 0..HEADS {
                let lo = head * HEAD_DIM;
                let hi = lo + HEAD_DIM;
                let dot_qk: f64 = q[lo..hi].iter().zip(k[lo..hi].iter()).map(|(a, b)| a * b).sum();
                scores[head * TOKEN_COUNT + t] = dot_qk * self.attn_scale;
            }
        }
        // Stable softmax over tokens per head; weighted sum of values into the context.
        let mut ctx = vec![0.0f64; EMBED_DIM];
        for head in 0..HEADS {
            let s = &scores[head * TOKEN_COUNT..(head + 1) * TOKEN_COUNT];
            let max_s = s.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let z: f64 = s.iter().map(|&x| (x - max_s).exp()).sum();
            for (t, &s_t) in s.iter().enumerate() {
                let a = (s_t - max_s).exp() / z;
                let base = t * EMBED_DIM + head * HEAD_DIM;
                for d in 0..HEAD_DIM {
                    ctx[head * HEAD_DIM + d] += a * v_all[base + d];
                }
            }
        }
        ctx
    }
}

impl Default for Cgt {
    /// CEP:WHAT: Default construction with the crate-default deterministic seed.
    /// CEP:WHY: Clippy new_without_default and idiomatic Default.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: same as Cgt::new.
    /// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
    fn default() -> Cgt {
        Cgt::new()
    }
}

/// CEP:WHAT: Validates the fixed-width feature vector: exact FEATURE_DIM length and finite
///   values.
/// CEP:WHY: NaN propagation is prevented at this boundary; the bounded-weight argument in the
///   module header then guarantees finiteness of all downstream math.
/// CEP:STATUS: complete
/// CEP:FAILURE: LengthMismatch or NonFiniteInput.
/// CEP:ASSUMES: none
/// CEP:COST: O(FEATURE_DIM), no allocation.
/// CEP:EVIDENCE: cgt::tests::{predict_rejects_nan_features, predict_rejects_wrong_feature_length}.
fn validate_features(features: &[f64]) -> Result<(), AutotunerError> {
    if features.len() != FEATURE_DIM {
        return Err(AutotunerError::LengthMismatch {
            expected: FEATURE_DIM,
            actual: features.len(),
        });
    }
    if features.iter().any(|f| !f.is_finite()) {
        return Err(AutotunerError::NonFiniteInput);
    }
    Ok(())
}

/// CEP:WHAT: Row-major matrix-vector product: `out[r] = sum_c w[r*cols + c] * x[c]`.
/// CEP:WHY: All projection matrices share one reviewed kernel.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; panics only on caller length bugs, which are internal invariants (private
///   fn, lengths fixed by named consts).
/// CEP:ASSUMES: w.len() == out.len() * cols and x.len() == cols.
/// CEP:COST: O(out.len() * cols).
/// CEP:EVIDENCE: exercised by every cgt::tests::predict_* test.
fn matvec(w: &[f64], cols: usize, x: &[f64], out: &mut [f64]) {
    for (r, o) in out.iter_mut().enumerate() {
        *o = w[r * cols..(r + 1) * cols].iter().zip(x.iter()).map(|(wv, xv)| wv * xv).sum();
    }
}

/// CEP:WHAT: Dot product of two equal-length slices.
/// CEP:WHY: Shared kernel for readout and gradient normalization.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (internal; lengths fixed by construction).
/// CEP:ASSUMES: equal lengths.
/// CEP:COST: O(n).
/// CEP:EVIDENCE: exercised by cgt::tests::fit_reduces_loss_on_repeated_sample.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// CEP:WHAT: One uniform draw in (-1, 1).
/// CEP:WHY: Shared init/mutation scale semantics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: next_f64 in [0, 1).
/// CEP:COST: one rng draw.
/// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
fn uniform_pm_one(rng: &mut SplitMix64) -> f64 {
    rng.next_f64() * 2.0 - 1.0
}

/// CEP:WHAT: Fills a buffer with uniform(-INIT_BOUND, INIT_BOUND) draws in index order.
/// CEP:WHY: Deterministic init requires a fixed draw order.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: one rng draw per element.
/// CEP:EVIDENCE: cgt::tests::determinism_across_seeded_instances.
fn fill_uniform(rng: &mut SplitMix64, out: &mut [f64]) {
    for v in out.iter_mut() {
        *v = uniform_pm_one(rng) * INIT_BOUND;
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Builds a valid genome with pseudo-random but seeded content.
    /// CEP:WHY: Test fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn fixture_genome(seed: u64) -> Genome {
        let mut rng = SplitMix64::seed_from_u64(seed);
        Genome {
            logits: (0..8).map(|_| rng.next_f64() * 2.0 - 1.0).collect(),
            iterations: (0..8).map(|_| 1 + (rng.next_f64() * 4.0) as u32).collect(),
            factors: (0..8).map(|_| rng.next_f64() * 2.0).collect(),
        }
    }

    /// CEP:WHAT: Builds a finite feature vector from a seed.
    /// CEP:WHY: Test fixture; layout [nodes|edges|loop depth|pressure].
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn fixture_features(seed: u64) -> Vec<f64> {
        let mut rng = SplitMix64::seed_from_u64(seed);
        (0..FEATURE_DIM).map(|_| rng.next_f64() * 2.0 - 1.0).collect()
    }

    /// CEP:WHAT: Predictions are finite, have non-negative variance, and are bit-identical across
    ///   repeated calls and across identically-seeded model instances.
    /// CEP:WHY: Determinism and float-hygiene acceptance criteria.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn predict_finite_stable_deterministic() {
        let model = Cgt::new();
        let features = fixture_features(11);
        let genome = fixture_genome(12);
        let p1 = model.predict(&features, &genome).unwrap();
        let p2 = model.predict(&features, &genome).unwrap();
        assert!(p1.delta_mean.is_finite() && p1.delta_var.is_finite() && p1.value.is_finite());
        assert!(p1.delta_var >= 0.0);
        assert_eq!(p1.delta_mean.to_bits(), p2.delta_mean.to_bits());
        assert_eq!(p1.delta_var.to_bits(), p2.delta_var.to_bits());
        assert_eq!(p1.value.to_bits(), p2.value.to_bits());
        let model_b = Cgt::new();
        let p3 = model_b.predict(&features, &genome).unwrap();
        assert_eq!(p1.delta_mean.to_bits(), p3.delta_mean.to_bits());
    }

    /// CEP:WHAT: Different init seeds produce different models (seeding actually matters).
    /// CEP:WHY: Guards against accidentally constant weights.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: two random 14.8k-weight inits collide with negligible probability.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn determinism_across_seeded_instances() {
        let a = Cgt::with_seed(999);
        let b = Cgt::with_seed(999);
        let c = Cgt::with_seed(1000);
        let features = fixture_features(5);
        let genome = fixture_genome(6);
        let pa = a.predict(&features, &genome).unwrap();
        let pb = b.predict(&features, &genome).unwrap();
        let pc = c.predict(&features, &genome).unwrap();
        assert_eq!(pa.delta_mean.to_bits(), pb.delta_mean.to_bits());
        assert_ne!(pa.delta_mean.to_bits(), pc.delta_mean.to_bits());
    }

    /// CEP:WHAT: NaN/infinite features are rejected with Err, never propagated.
    /// CEP:WHY: Float hygiene hard rule.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn predict_rejects_nan_features() {
        let model = Cgt::new();
        let genome = fixture_genome(1);
        let mut features = fixture_features(2);
        features[3] = f64::NAN;
        assert_eq!(model.predict(&features, &genome).unwrap_err(), AutotunerError::NonFiniteInput);
        features[3] = f64::INFINITY;
        assert_eq!(model.predict(&features, &genome).unwrap_err(), AutotunerError::NonFiniteInput);
    }

    /// CEP:WHAT: Wrong-length features are rejected with LengthMismatch.
    /// CEP:WHY: FEATURE_DIM is a hard contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn predict_rejects_wrong_feature_length() {
        let model = Cgt::new();
        let genome = fixture_genome(1);
        let features = vec![0.0; FEATURE_DIM + 1];
        assert!(matches!(
            model.predict(&features, &genome),
            Err(AutotunerError::LengthMismatch { .. })
        ));
    }

    /// CEP:WHAT: An invalid genome (empty logits) is rejected, not panicking.
    /// CEP:WHY: Empty-logit handling must be Err per the failure contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn predict_rejects_invalid_genome() {
        let model = Cgt::new();
        let features = fixture_features(3);
        let empty = Genome::default();
        assert_eq!(model.predict(&features, &empty).unwrap_err(), AutotunerError::EmptyInput);
    }

    /// CEP:WHAT: Repeated fit on one identical sample decreases the returned loss monotonically
    ///   (bounded by the contraction argument) and keeps every value finite.
    /// CEP:WHY: The online-learning acceptance criterion; the normalized gradient makes the
    ///   per-step contraction provable rather than incidental.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: readout-only SGD with LEARNING_RATE in (0,1).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn fit_reduces_loss_on_repeated_sample() {
        let mut model = Cgt::new();
        let features = fixture_features(21);
        let genome = fixture_genome(22);
        let target = 0.5;
        let mut losses = Vec::new();
        for _ in 0..200 {
            let loss = model.fit(&features, &genome, target).unwrap();
            assert!(loss.is_finite() && loss >= 0.0);
            losses.push(loss);
        }
        for w in losses.windows(2) {
            assert!(w[1] <= w[0] + 1e-18, "loss must be non-increasing: {} -> {}", w[0], w[1]);
        }
        assert!(losses[199] < losses[0] * 0.5, "200 steps must halve the initial loss at least");
        assert!(losses[199] < losses[0], "loss strictly decreased overall");
    }

    /// CEP:WHAT: fit rejects non-finite observed deltas.
    /// CEP:WHY: NaN targets must fail closed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn fit_rejects_non_finite_target() {
        let mut model = Cgt::new();
        let features = fixture_features(31);
        let genome = fixture_genome(32);
        assert_eq!(
            model.fit(&features, &genome, f64::NAN).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
    }

    /// CEP:WHAT: Dropout variance is non-negative across several genomes and differs between
    ///   distinct genomes (the uncertainty signal carries information).
    /// CEP:WHY: MC-dropout epistemic proxy contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn dropout_variance_nonneg() {
        let model = Cgt::new();
        let features = fixture_features(41);
        for seed in 0..8u64 {
            let p = model.predict(&features, &fixture_genome(100 + seed)).unwrap();
            assert!(p.delta_var >= 0.0);
        }
        let pa = model.predict(&features, &fixture_genome(200)).unwrap();
        let pb = model.predict(&features, &fixture_genome(201)).unwrap();
        assert_ne!(pa.delta_var.to_bits(), pb.delta_var.to_bits());
    }

    /// CEP:WHAT: The feature layout constant sums to FEATURE_DIM and splits into whole tokens.
    /// CEP:WHY: Layout documentation is enforced, not just claimed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn feature_layout_sums() {
        assert_eq!(FEATURE_DIM, 14);
        assert_eq!(TOKEN_COUNT * TOKEN_DIM, FEATURE_DIM);
        assert_eq!(HEAD_DIM * HEADS, EMBED_DIM);
    }
}
