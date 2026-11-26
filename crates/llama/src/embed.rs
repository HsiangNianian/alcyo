//! An embedding model run inside the app through libllama itself, for meeting search on the iPad.
//!
//! The desktop asks a `llama-server --embeddings` of its own, but the iPad can start no other program, and the
//! server built into the app ([`crate::Server`]) keeps its state in globals, so there is only one, and that one is
//! the board model's. libllama, on the other hand, holds any number of models and contexts in one process, so the
//! search model is loaded beside it from the same libraries and asked directly, on the CPU only: the graphics
//! card stays the board model's, and a few short passages at a time take well under a second there.

// Without libllama nothing here is ever made, so most of it goes unused.
#![cfg_attr(llama_missing, allow(dead_code))]

#[cfg(not(llama_missing))]
use anyhow::ensure;
use anyhow::{bail, Context, Result};
use std::path::Path;

/// How many tokens a passage may have, and how many go to the model at once. Passages are a few hundred
/// characters, well within it; a longer one is cut (see [`fit`]). What the context costs grows with this.
const CONTEXT: u32 = 1024;
/// How many passages share one pass through the model.
const SEQUENCES: u32 = 16;
/// Threads at most: the search runs beside listening and the board model, and is not in a hurry.
const THREADS: usize = 4;

/// What the model costs beyond its file, measured with embeddinggemma-300M on the CPU (examples/embed.rs): about
/// 60 MiB once loaded, for the context's buffers and libllama's own, and 90 MiB at most while embedding.
const OVERHEAD: u64 = 128 << 20;
/// What is left untouched besides: the board model and the interface grow while the search model works, and
/// iPadOS ends the app at its limit without a word.
const MARGIN: u64 = 256 << 20;

/// How much memory loading a model file of `model_bytes` takes. The weights are mapped from the file, but where
/// the CPU has a faster layout for them libllama copies them into the app's own memory, so the whole file counts,
/// and the context besides.
pub fn embedder_need(model_bytes: u64) -> u64 {
    model_bytes.saturating_add(OVERHEAD)
}

/// Whether a model file of `model_bytes` may be loaded with `available` bytes left before the system ends the app
/// ([`crate::available_memory`]): its need and a margin of 256 MiB besides. Where that is not known (any system but the
/// iPad's) it is let through.
pub fn fits(model_bytes: u64, available: Option<u64>) -> bool {
    available.is_none_or(|left| left >= embedder_need(model_bytes).saturating_add(MARGIN))
}

/// The token ids of a text cut to at most `limit`, keeping the last, which is the end-of-text token the model
/// pools up to when the vocabulary adds one.
fn fit(mut tokens: Vec<i32>, limit: usize) -> Vec<i32> {
    if tokens.len() > limit && limit > 0 {
        let last = tokens[tokens.len() - 1];
        tokens.truncate(limit - 1);
        tokens.push(last);
    }
    tokens
}

/// Texts in groups that each go to the model at once: at most `sequences` texts and `tokens` tokens together, in
/// order. Each text is within `tokens` already.
fn groups(lengths: &[usize], tokens: usize, sequences: usize) -> Vec<std::ops::Range<usize>> {
    let mut out = vec![];
    let (mut start, mut used) = (0, 0);
    for (i, &len) in lengths.iter().enumerate() {
        if i > start && (used + len > tokens || i - start >= sequences) {
            out.push(start..i);
            (start, used) = (i, 0);
        }
        used += len;
    }
    if start < lengths.len() {
        out.push(start..lengths.len());
    }
    out
}

#[cfg(not(llama_missing))]
mod ffi {
    //! What of llama.h is used, mirrored from our fork at commit db229e2c3ab18ed12ee32a1e0178fdf6b91a6422 (tag
    //! alcyo-b11954-2, the libraries crates/llama links). The parameter structs are passed and returned by value,
    //! so their fields must match that header's exactly, in order and type; a newer llama.cpp needs them checked
    //! again. Their sizes on a 64-bit system are asserted below, from a C program built against the header.
    #![allow(non_camel_case_types, dead_code)]

    use std::ffi::{c_char, c_void};

    pub type llama_token = i32;
    pub type llama_pos = i32;
    pub type llama_seq_id = i32;

    pub const POOLING_UNSPECIFIED: i32 = -1;
    pub const POOLING_NONE: i32 = 0;
    pub const POOLING_RANK: i32 = 4;

    #[repr(C)]
    pub struct llama_model_params {
        pub devices: *mut *mut c_void,
        pub tensor_buft_overrides: *const c_void,
        pub n_gpu_layers: i32,
        pub split_mode: i32,
        pub load_mode: i32,
        pub lazy_mode: i32,
        pub main_gpu: i32,
        pub tensor_split: *const f32,
        pub progress_callback: Option<unsafe extern "C" fn(progress: f32, user_data: *mut c_void) -> bool>,
        pub progress_callback_user_data: *mut c_void,
        pub kv_overrides: *const c_void,
        pub vocab_only: bool,
        pub check_tensors: bool,
        pub use_extra_bufts: bool,
        pub no_host: bool,
        pub no_alloc: bool,
        pub load_mtp: bool,
    }

    #[repr(C)]
    pub struct llama_context_params {
        pub n_ctx: u32,
        pub n_batch: u32,
        pub n_ubatch: u32,
        pub n_seq_max: u32,
        pub n_rs_seq: u32,
        pub n_outputs_max: u32,
        pub n_outputs_max_per_seq: u32,
        pub n_threads: i32,
        pub n_threads_batch: i32,
        pub ctx_type: i32,
        pub rope_scaling_type: i32,
        pub pooling_type: i32,
        pub attention_type: i32,
        pub flash_attn_type: i32,
        pub rope_freq_base: f32,
        pub rope_freq_scale: f32,
        pub yarn_ext_factor: f32,
        pub yarn_attn_factor: f32,
        pub yarn_beta_fast: f32,
        pub yarn_beta_slow: f32,
        pub yarn_orig_ctx: u32,
        pub defrag_thold: f32,
        pub cb_eval: Option<unsafe extern "C" fn(tensor: *mut c_void, ask: bool, user_data: *mut c_void) -> bool>,
        pub cb_eval_user_data: *mut c_void,
        pub type_k: i32,
        pub type_v: i32,
        pub moe_cache_size: usize,
        pub abort_callback: Option<unsafe extern "C" fn(data: *mut c_void) -> bool>,
        pub abort_callback_data: *mut c_void,
        pub embeddings: bool,
        pub offload_kqv: bool,
        pub no_perf: bool,
        pub op_offload: bool,
        pub swa_full: bool,
        pub kv_unified: bool,
        pub samplers: *mut c_void,
        pub n_samplers: usize,
        pub ctx_other: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct llama_batch {
        pub n_tokens: i32,
        pub token: *mut llama_token,
        pub embd: *mut f32,
        pub pos: *mut llama_pos,
        pub n_seq_id: *mut i32,
        pub seq_id: *mut *mut llama_seq_id,
        pub logits: *mut i8,
    }

    #[cfg(target_pointer_width = "64")]
    const _: () = {
        use std::mem::{offset_of, size_of};
        assert!(size_of::<llama_model_params>() == 80);
        assert!(offset_of!(llama_model_params, tensor_split) == 40);
        assert!(offset_of!(llama_model_params, vocab_only) == 72);
        assert!(size_of::<llama_context_params>() == 168);
        assert!(offset_of!(llama_context_params, cb_eval) == 88);
        assert!(offset_of!(llama_context_params, moe_cache_size) == 112);
        assert!(offset_of!(llama_context_params, embeddings) == 136);
        assert!(offset_of!(llama_context_params, ctx_other) == 160);
        assert!(size_of::<llama_batch>() == 56);
    };

    pub enum llama_model {}
    pub enum llama_context {}
    pub enum llama_vocab {}
    pub enum llama_memory {}

    extern "C" {
        pub fn llama_backend_init();
        pub fn llama_model_default_params() -> llama_model_params;
        pub fn llama_context_default_params() -> llama_context_params;
        pub fn llama_model_load_from_file(path: *const c_char, params: llama_model_params) -> *mut llama_model;
        pub fn llama_model_free(model: *mut llama_model);
        pub fn llama_init_from_model(model: *mut llama_model, params: llama_context_params) -> *mut llama_context;
        pub fn llama_free(ctx: *mut llama_context);
        pub fn llama_model_get_vocab(model: *const llama_model) -> *const llama_vocab;
        pub fn llama_model_n_ctx_train(model: *const llama_model) -> i32;
        pub fn llama_model_n_embd_out(model: *const llama_model) -> i32;
        pub fn llama_model_has_encoder(model: *const llama_model) -> bool;
        pub fn llama_model_has_decoder(model: *const llama_model) -> bool;
        pub fn llama_n_batch(ctx: *const llama_context) -> u32;
        pub fn llama_n_ubatch(ctx: *const llama_context) -> u32;
        pub fn llama_n_seq_max(ctx: *const llama_context) -> u32;
        pub fn llama_pooling_type(ctx: *const llama_context) -> i32;
        pub fn llama_get_memory(ctx: *const llama_context) -> *mut llama_memory;
        pub fn llama_memory_clear(mem: *mut llama_memory, data: bool);
        pub fn llama_tokenize(
            vocab: *const llama_vocab,
            text: *const c_char,
            text_len: i32,
            tokens: *mut llama_token,
            n_tokens_max: i32,
            add_special: bool,
            parse_special: bool,
        ) -> i32;
        pub fn llama_batch_init(n_tokens: i32, embd: i32, n_seq_max: i32) -> llama_batch;
        pub fn llama_batch_free(batch: llama_batch);
        pub fn llama_encode(ctx: *mut llama_context, batch: llama_batch) -> i32;
        pub fn llama_decode(ctx: *mut llama_context, batch: llama_batch) -> i32;
        pub fn llama_get_embeddings_seq(ctx: *mut llama_context, seq_id: llama_seq_id) -> *mut f32;
    }
}

/// An embedding model loaded in this process. It is used from one thread at a time, and frees all it holds
/// when dropped.
pub struct Embedder {
    #[cfg(not(llama_missing))]
    model: *mut ffi::llama_model,
    #[cfg(not(llama_missing))]
    ctx: *mut ffi::llama_context,
    #[cfg(not(llama_missing))]
    batch: ffi::llama_batch,
    /// The most tokens in one pass, and so in one passage.
    tokens: usize,
    /// The most passages in one pass.
    sequences: usize,
    /// The size of a vector.
    dims: usize,
}

// SAFETY: libllama's model and context are not tied to the thread that made them; `embed` takes `&mut self`, so
// only one thread uses them at a time, and nothing here is shared.
unsafe impl Send for Embedder {}

impl Embedder {
    /// Load the embedding model in `model`, on the CPU. Fails where this build has no libllama, where the system
    /// says there is not memory enough for it ([`fits`]), and where the file is not an embedding model libllama
    /// can pool.
    pub fn load(model: &Path) -> Result<Self> {
        if !crate::BUILT_IN {
            bail!("this build has no local model built in");
        }
        let size = std::fs::metadata(model).with_context(|| format!("reading {}", model.display()))?.len();
        if !fits(size, crate::available_memory()) {
            bail!(
                "not enough memory for the search model: it needs about {} MB, and {} MB are left",
                (embedder_need(size) + MARGIN) >> 20,
                crate::available_memory().unwrap_or(0) >> 20
            );
        }
        Self::load_unchecked(model)
    }

    #[cfg(llama_missing)]
    fn load_unchecked(_: &Path) -> Result<Self> {
        bail!("this build has no local model built in")
    }

    #[cfg(not(llama_missing))]
    fn load_unchecked(path: &Path) -> Result<Self> {
        use std::ffi::CString;
        use std::sync::Once;

        // It may be called more than once, as the built-in server does too, but there is no need.
        static INIT: Once = Once::new();
        // SAFETY: no arguments; it sets up libllama's tables and finds its backends.
        INIT.call_once(|| unsafe { ffi::llama_backend_init() });

        let c_path = CString::new(path.to_string_lossy().as_bytes()).context("the model's path holds a NUL byte")?;
        // An empty list of devices keeps every weight and every step on the CPU, as `--device none` does.
        let mut no_devices: [*mut std::ffi::c_void; 1] = [std::ptr::null_mut()];
        // SAFETY: returns a plain struct by value, mirrored in `ffi`.
        let mut params = unsafe { ffi::llama_model_default_params() };
        params.devices = no_devices.as_mut_ptr();
        params.n_gpu_layers = 0;
        // The weights stay mapped from the file rather than copied into a faster layout in the app's own memory, as
        // libllama would on an ARM CPU: on the iPad that copy would count against the app's limit beside the board
        // model, and searching a few passages does not need the speed.
        params.use_extra_bufts = false;
        // SAFETY: the path is a C string and the device list ends in null; both outlive the call.
        let model = unsafe { ffi::llama_model_load_from_file(c_path.as_ptr(), params) };
        if model.is_null() {
            bail!("could not load the search model from {}", path.display());
        }
        // From here on, dropping `this` frees what has been made.
        let mut this = Self { model, ctx: std::ptr::null_mut(), batch: empty_batch(), tokens: 0, sequences: 0, dims: 0 };

        // SAFETY: the model is loaded and alive.
        let (has_encoder, has_decoder, trained, dims) = unsafe {
            (ffi::llama_model_has_encoder(model), ffi::llama_model_has_decoder(model), ffi::llama_model_n_ctx_train(model), ffi::llama_model_n_embd_out(model))
        };
        ensure!(!(has_encoder && has_decoder), "{} is an encoder-decoder model, which gives no embeddings", path.display());
        ensure!(dims > 0, "{} gives no vectors", path.display());
        let context = if trained > 0 { CONTEXT.min(trained as u32) } else { CONTEXT };
        let threads = std::thread::available_parallelism().map_or(THREADS, |n| n.get().min(THREADS)) as i32;

        // SAFETY: as above.
        let mut cp = unsafe { ffi::llama_context_default_params() };
        cp.embeddings = true;
        // Every passage is read whole in one step, as a model that looks both ways needs, so the step is as
        // large as the batch (examples/embedding/embedding.cpp does the same).
        (cp.n_ctx, cp.n_batch, cp.n_ubatch, cp.n_seq_max) = (context, context, context, SEQUENCES);
        // The passages of a pass share the context rather than each getting a sixteenth of it, so any one of them
        // may be as long as the whole.
        cp.kv_unified = true;
        (cp.n_threads, cp.n_threads_batch) = (threads, threads);
        // The model says how it pools, as it does to llama-server.
        cp.pooling_type = ffi::POOLING_UNSPECIFIED;
        // Nothing goes to the graphics card, also not the attention or operations libllama would move there.
        (cp.offload_kqv, cp.op_offload) = (false, false);
        // SAFETY: the model is alive, and the context is freed before it (Drop).
        this.ctx = unsafe { ffi::llama_init_from_model(model, cp) };
        if this.ctx.is_null() {
            bail!("could not make a context for the search model");
        }
        // SAFETY: the context is alive.
        let (batch, ubatch, sequences, pooling) =
            unsafe { (ffi::llama_n_batch(this.ctx), ffi::llama_n_ubatch(this.ctx), ffi::llama_n_seq_max(this.ctx), ffi::llama_pooling_type(this.ctx)) };
        ensure!(pooling != ffi::POOLING_NONE && pooling != ffi::POOLING_RANK, "{} does not pool its tokens into one vector", path.display());
        this.tokens = batch.min(ubatch) as usize;
        this.sequences = sequences.max(1) as usize;
        this.dims = dims as usize;
        // SAFETY: plain allocations, freed in Drop; each token belongs to one sequence.
        this.batch = unsafe { ffi::llama_batch_init(this.tokens as i32, 0, 1) };
        Ok(this)
    }

    /// The size of the vectors it gives.
    pub fn dims(&self) -> usize {
        self.dims
    }

    /// One vector for each text, of the model's full size and as the model pools it, unscaled. A text longer than
    /// the model reads at once is cut at its end.
    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_impl(texts)
    }

    #[cfg(llama_missing)]
    fn embed_impl(&mut self, _: &[String]) -> Result<Vec<Vec<f32>>> {
        bail!("this build has no local model built in")
    }

    #[cfg(not(llama_missing))]
    fn embed_impl(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let tokenized: Vec<Vec<i32>> = texts.iter().map(|t| self.tokenize(t).map(|ids| fit(ids, self.tokens))).collect::<Result<_>>()?;
        let lengths: Vec<usize> = tokenized.iter().map(Vec::len).collect();
        let mut out = Vec::with_capacity(texts.len());
        for group in groups(&lengths, self.tokens, self.sequences) {
            let mut n = 0usize;
            for (seq, ids) in tokenized[group.clone()].iter().enumerate() {
                for (pos, &id) in ids.iter().enumerate() {
                    debug_assert!(n < self.tokens, "a group is within the batch");
                    // SAFETY: `n` is below the batch's size (`groups` keeps each group within `self.tokens`), and
                    // every array of it was allocated for that many tokens, with room for one sequence each.
                    unsafe {
                        *self.batch.token.add(n) = id;
                        *self.batch.pos.add(n) = pos as i32;
                        *self.batch.n_seq_id.add(n) = 1;
                        **self.batch.seq_id.add(n) = seq as i32;
                        *self.batch.logits.add(n) = 1;
                    }
                    n += 1;
                }
            }
            if n == 0 {
                // Only empty texts, which tokenize to nothing where the model adds no special tokens.
                out.extend(group.map(|_| vec![0.0; self.dims]));
                continue;
            }
            self.batch.n_tokens = n as i32;
            // SAFETY: the context is alive, and the batch filled up to `n_tokens` above. A model that looks both
            // ways, as EmbeddingGemma, has no memory and is encoded; one that reads forward only, as
            // Qwen3-Embedding, has its memory cleared so each pass starts afresh.
            let code = unsafe {
                let memory = ffi::llama_get_memory(self.ctx);
                if memory.is_null() {
                    ffi::llama_encode(self.ctx, self.batch)
                } else {
                    ffi::llama_memory_clear(memory, true);
                    ffi::llama_decode(self.ctx, self.batch)
                }
            };
            ensure!(code == 0, "the search model failed on a batch of {} passages ({code})", group.len());
            for (seq, ids) in tokenized[group].iter().enumerate() {
                if ids.is_empty() {
                    out.push(vec![0.0; self.dims]);
                    continue;
                }
                // SAFETY: the context is alive and just pooled this sequence; the vector it points to holds the
                // model's output size and stays valid until the next pass, so it is copied now.
                let v = unsafe { ffi::llama_get_embeddings_seq(self.ctx, seq as i32) };
                ensure!(!v.is_null(), "the search model gave no vector for a passage");
                out.push(unsafe { std::slice::from_raw_parts(v, self.dims) }.to_vec());
            }
        }
        Ok(out)
    }

    /// A text's token ids, with the special tokens the model adds, as llama-server reads them.
    #[cfg(not(llama_missing))]
    fn tokenize(&self, text: &str) -> Result<Vec<i32>> {
        let len = i32::try_from(text.len()).context("a passage is too long")?;
        // SAFETY: the model is alive while `self` is.
        let vocab = unsafe { ffi::llama_model_get_vocab(self.model) };
        let mut ids = vec![0i32; text.len() + 8];
        for _ in 0..2 {
            // SAFETY: the text is `len` bytes, and `ids` has room for the count given.
            let n = unsafe { ffi::llama_tokenize(vocab, text.as_ptr().cast(), len, ids.as_mut_ptr(), ids.len() as i32, true, true) };
            if n >= 0 {
                ids.truncate(n as usize);
                return Ok(ids);
            }
            if n == i32::MIN {
                bail!("a passage has too many tokens");
            }
            // Too small: it says how many it needs.
            ids.resize(n.unsigned_abs() as usize, 0);
        }
        bail!("could not read a passage into tokens")
    }
}

#[cfg(not(llama_missing))]
fn empty_batch() -> ffi::llama_batch {
    use std::ptr::null_mut;
    ffi::llama_batch { n_tokens: 0, token: null_mut(), embd: null_mut(), pos: null_mut(), n_seq_id: null_mut(), seq_id: null_mut(), logits: null_mut() }
}

impl Drop for Embedder {
    fn drop(&mut self) {
        // SAFETY: each was made by libllama and is freed once, the context before the model it uses; a batch
        // never allocated holds only nulls, which llama_batch_free passes over.
        #[cfg(not(llama_missing))]
        unsafe {
            ffi::llama_batch_free(self.batch);
            self.batch = empty_batch();
            if !self.ctx.is_null() {
                ffi::llama_free(self.ctx);
            }
            if !self.model.is_null() {
                ffi::llama_model_free(self.model);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_gate_leaves_room_besides_the_model() {
        let model = 318 << 20;
        assert!(fits(model, None), "not known: let through");
        assert!(!fits(model, Some(model)), "the file alone is not enough");
        assert!(!fits(model, Some(embedder_need(model))), "nor is the bare need");
        assert!(fits(model, Some(embedder_need(model) + MARGIN)));
        assert!(!fits(u64::MAX, Some(u64::MAX - 1)), "no overflow");
    }

    #[test]
    fn a_long_text_is_cut_keeping_its_last_token() {
        assert_eq!(fit(vec![1, 2, 3], 5), vec![1, 2, 3]);
        assert_eq!(fit(vec![2, 10, 11, 12, 13, 1], 4), vec![2, 10, 11, 1]);
        assert_eq!(fit(vec![], 4), Vec::<i32>::new());
    }

    #[test]
    fn texts_are_grouped_within_the_batch() {
        assert_eq!(groups(&[], 10, 4), vec![]);
        assert_eq!(groups(&[3, 3, 3, 3], 10, 4), vec![0..3, 3..4]);
        assert_eq!(groups(&[10, 1, 10], 10, 4), vec![0..1, 1..2, 2..3]);
        assert_eq!(groups(&[1; 5], 100, 2), vec![0..2, 2..4, 4..5]);
        for g in groups(&[7, 2, 9, 1, 1, 5, 10, 3], 10, 3) {
            assert!(g.len() <= 3 && [7, 2, 9, 1, 1, 5, 10, 3][g].iter().sum::<usize>() <= 10);
        }
    }

    /// The search model's file: ALCYO_SEARCH_MODEL, or where the evaluation or the app's resources keep it.
    fn search_model() -> Option<std::path::PathBuf> {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        std::env::var_os("ALCYO_SEARCH_MODEL")
            .map(Into::into)
            .into_iter()
            .chain(["eval/search/.models", "resources/search"].map(|dir| root.join(dir).join("embeddinggemma-300M-Q8_0.gguf")))
            .find(|p| p.is_file())
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (norm(a) * norm(b))
    }

    /// The search model run in this process, where libllama is linked (ALCYO_LLAMA_LIB_DIR) and the model is
    /// here: a paraphrase is nearer than something unrelated, also across languages, and a passage reads the same
    /// alone as among others. Skipped otherwise.
    #[test]
    fn a_paraphrase_is_nearer_than_something_else() {
        let Some(model) = search_model().filter(|_| crate::BUILT_IN) else {
            eprintln!("skipped: no libllama or no search model here");
            return;
        };
        let mut embedder = Embedder::load(&model).expect("the search model loads");
        let texts: Vec<String> = [
            "task: search result | query: when are we shipping now?",
            "title: Release review | text: We moved the ship date from Friday to Tuesday.",
            "title: Lunch | text: The canteen serves noodles on Wednesdays.",
            "title: 发布评审 | text: 我们把发布日期从周五改到了周二。",
            "",
        ]
        .map(String::from)
        .into();
        let vectors = embedder.embed(&texts).unwrap();
        assert_eq!(vectors.len(), texts.len());
        assert!(vectors.iter().all(|v| v.len() == embedder.dims()));
        let (near, far, across) = (cosine(&vectors[0], &vectors[1]), cosine(&vectors[0], &vectors[2]), cosine(&vectors[0], &vectors[3]));
        assert!(near > far + 0.1, "paraphrase {near}, unrelated {far}");
        assert!(across > far + 0.1, "across languages {across}, unrelated {far}");
        let alone = embedder.embed(&texts[1..2]).unwrap();
        assert!(cosine(&alone[0], &vectors[1]) > 0.999, "the same alone as in a batch");
        // Longer than the model reads at once: cut, not refused, and many passages fill several passes.
        let long = vec!["the budget and the schedule ".repeat(400); 3];
        assert_eq!(embedder.embed(&long).unwrap().len(), 3);
        let many: Vec<String> = (0..40).map(|i| format!("title: none | text: item {i} of the plan")).collect();
        assert_eq!(embedder.embed(&many).unwrap().len(), 40);
    }

    #[test]
    fn without_libllama_loading_says_so() {
        if !crate::BUILT_IN {
            let err = Embedder::load(Path::new(file!())).err().expect("no libllama here");
            assert!(format!("{err:#}").contains("no local model"), "{err:#}");
        }
    }
}
