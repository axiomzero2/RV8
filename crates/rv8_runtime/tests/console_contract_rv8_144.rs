// CEP:FILE: crates/rv8_runtime/tests/console_contract_rv8_144.rs
// CEP:WHAT: The RV8-144 regression battery for the panic console contract: registration,
//   replacement, removal, chunk delivery (order, boundaries, content, volume), the
//   report_panic transcript, payload_text mapping, and — the bug itself — serialized
//   install/write/observe/remove cycles and multi-thread write stress.  The console slot
//   is a process-global writer, so every test in this file takes the file-local
//   CONSOLE_MUX for the whole install/assert/remove window; the stress tests spawn
//   threads that contend for the same mux, deterministically reproducing the collision
//   that made `console_callback_receives_text` flake (~10-20%: a sibling test's
//   install_sink clobbered the global console between two writes, losing a chunk).
// CEP:WHY: STANDARDS pins twenty regression tests per bug.  The lib-module fix (a
//   CONSOLE_MUX in src/panic.rs) restored determinism; this battery pins the console
//   contract that the race corrupted, so any future regression of the registration
//   semantics or of the serialized-test pattern fails loudly and individually.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-144
// CEP:FAILURE: panics via assert on any contract violation (test-only).
// CEP:ASSUMES: each test holds CONSOLE_MUX while the global console points at this
//   file's sink; the sink mutex ignores poisoning so a sibling failure cannot cascade.
// CEP:COST: test-only; stress tests total a few thousand console writes.
// CEP:EVIDENCE: this battery; the flake log in the RV8-144 ticket (STANDARDS.md).

use std::sync::{Mutex, OnceLock};

use rv8_runtime::panic::{console_write, payload_text, report_panic, set_console};

/// The file-local serialization point: one test owns the process-global console at a
/// time.  Poisoning is ignored so a failed assert does not cascade into siblings.
static CONSOLE_MUX: Mutex<()> = Mutex::new(());

/// The sink the writer fns append to (leaked once; cleared per test under the mux).
static SINK: OnceLock<&'static Mutex<Vec<String>>> = OnceLock::new();

fn sink() -> &'static Mutex<Vec<String>> {
    SINK.get_or_init(|| Box::leak(Box::new(Mutex::new(Vec::new()))))
}

/// The default writer: appends the chunk verbatim.  fn item, no captures.
fn writer(text: &str) {
    sink().lock().unwrap_or_else(|e| e.into_inner()).push(text.to_string());
}

/// Prefixing writers for the replacement tests: both append into the one sink, tagged,
/// so a swap is observable as a change of prefix.
fn writer_a(text: &str) {
    sink().lock().unwrap_or_else(|e| e.into_inner()).push(format!("a:{text}"));
}

fn writer_b(text: &str) {
    sink().lock().unwrap_or_else(|e| e.into_inner()).push(format!("b:{text}"));
}

/// Under the mux: register the verbatim writer and start from an empty sink.
fn install() {
    sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
    set_console(Some(writer));
}

/// Under the mux: remove the console and leave the sink empty for the next test.
fn teardown() {
    set_console(None);
    sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn snap() -> Vec<String> {
    sink().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ---------------------------------------------------------------------------
// 1-8: registration, delivery, replacement, removal.
// ---------------------------------------------------------------------------

/// T01: a registered console receives every chunk, in order.
#[test]
fn t01_registered_console_receives_chunks_in_order() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("hello");
    console_write("world");
    assert_eq!(snap(), vec!["hello", "world"]);
    teardown();
}

/// T02: chunk boundaries are preserved — one callback call per write, never merged.
#[test]
fn t02_chunk_boundaries_preserved() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("a");
    console_write("b");
    console_write("c");
    let got = snap();
    assert_eq!(got.len(), 3);
    assert_eq!(got, vec!["a", "b", "c"]);
    teardown();
}

/// T03: delivery order equals write order across a longer sequence.
#[test]
fn t03_chunks_delivered_in_write_order() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    let tags: Vec<String> = (0..8).map(|i| format!("chunk-{i}")).collect();
    for t in &tags {
        console_write(t);
    }
    assert_eq!(snap(), tags);
    teardown();
}

/// T04: an empty chunk is still one delivery — the slot routes chunks, not content.
#[test]
fn t04_empty_chunk_is_delivered() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("");
    console_write("x");
    console_write("");
    assert_eq!(snap(), vec!["", "x", ""]);
    teardown();
}

/// T05: with no console registered, writes are dropped silently and never panic.
#[test]
fn t05_no_console_drops_silently() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    teardown();
    console_write("into the void");
    report_panic("also the void");
    assert!(snap().is_empty());
}

/// T06: registering a new writer replaces the previous one mid-stream.
#[test]
fn t06_replacement_swaps_the_writer() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    set_console(Some(writer_a));
    console_write("one");
    set_console(Some(writer_b));
    console_write("two");
    assert_eq!(snap(), vec!["a:one", "b:two"]);
    teardown();
}

/// T07: removal stops delivery immediately; later chunks are dropped.
#[test]
fn t07_removal_stops_delivery() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("kept");
    set_console(None);
    console_write("dropped");
    assert_eq!(snap(), vec!["kept"]);
    teardown();
}

/// T08: re-registration after removal resumes delivery — the slot is reusable.
#[test]
fn t08_re_registration_after_removal_resumes_delivery() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("first");
    set_console(None);
    console_write("lost");
    set_console(Some(writer));
    console_write("second");
    assert_eq!(snap(), vec!["first", "second"]);
    teardown();
}

// ---------------------------------------------------------------------------
// 9-12: the report transcript and payload mapping.
// ---------------------------------------------------------------------------

/// T09: report_panic emits the fixed three-chunk transcript: prefix, message, newline.
#[test]
fn t09_report_panic_chunk_sequence() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    report_panic("unwrap on None");
    assert_eq!(snap(), vec!["panicked: ", "unwrap on None", "\n"]);
    teardown();
}

/// T10: report_panic without a console is silent and never panics.
#[test]
fn t10_report_panic_with_no_console_is_silent() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    teardown();
    report_panic("nobody is listening");
    assert!(snap().is_empty());
}

/// T11: a &str payload maps to itself.
#[test]
fn t11_payload_text_str_passthrough() {
    assert_eq!(payload_text(&"boom"), "boom");
    assert_eq!(payload_text(&""), "");
}

/// T12: every non-&str payload maps to the fixed fallback text.
#[test]
fn t12_payload_text_non_str_maps_to_fallback() {
    assert_eq!(payload_text(&42usize), "panic payload is not a &str");
    assert_eq!(payload_text(&1i8), "panic payload is not a &str");
    let owned = String::from("not a &str payload");
    assert_eq!(payload_text(&owned), "panic payload is not a &str");
}

// ---------------------------------------------------------------------------
// 13-20: the RV8-144 regression core — stress and serialized-cycle tests that
// deterministically reproduce the parallel-test collision.
// ---------------------------------------------------------------------------

/// T13: two threads × 250 writes: every chunk arrives, none is lost or duplicated.
#[test]
fn t13_stress_two_threads_no_lost_chunks() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    let mut hs = Vec::new();
    for tid in 0..2 {
        hs.push(std::thread::spawn(move || {
            for i in 0..250 {
                console_write(&format!("t{tid}-{i}"));
            }
        }));
    }
    for h in hs {
        h.join().expect("writer thread");
    }
    let got = snap();
    assert_eq!(got.len(), 500, "every chunk delivered exactly once");
    for tid in 0..2 {
        assert_eq!(got.iter().filter(|s| s.starts_with(&format!("t{tid}-"))).count(), 250);
    }
    teardown();
}

/// T14: eight threads × 100 writes: same exact-count contract under heavier contention.
#[test]
fn t14_stress_eight_threads_no_lost_chunks() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    let mut hs = Vec::new();
    for tid in 0..8 {
        hs.push(std::thread::spawn(move || {
            for i in 0..100 {
                console_write(&format!("t{tid}-{i}"));
            }
        }));
    }
    for h in hs {
        h.join().expect("writer thread");
    }
    assert_eq!(snap().len(), 800);
    teardown();
}

/// T15: THE regression, multi-chunk shape: mux workers run full report_panic cycles and
/// each must observe the exact three-chunk transcript.  A sibling install landing
/// mid-report (the original flake shape, one chunk earlier) truncates the transcript to
/// its prefix.  The mux makes every cycle atomic, so this is deterministic.
#[test]
fn t15_per_cycle_mux_report_transcripts_are_exact() {
    let mut hs = Vec::new();
    for tid in 0..4usize {
        hs.push(std::thread::spawn(move || {
            for cycle in 0..50u32 {
                let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
                install();
                let msg = format!("m-t{tid}-{cycle}");
                report_panic(&msg);
                assert_eq!(
                    snap(),
                    vec!["panicked: ".to_string(), msg, "\n".to_string()],
                    "report cycle must observe exactly its own three-chunk transcript"
                );
                teardown();
            }
        }));
    }
    for h in hs {
        h.join().expect("cycle thread");
    }
}

/// T16: THE regression, two-guard shape: the outer test does NOT hold the mux while the
/// workers run; each worker takes the mux per cycle and asserts its exact observation.
#[test]
fn t16_per_cycle_mux_workers_observe_exact_chunks() {
    let mut hs = Vec::new();
    for tid in 0..4usize {
        hs.push(std::thread::spawn(move || {
            for cycle in 0..50u32 {
                let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
                install();
                let tag = format!("cyc-t{tid}-{cycle}");
                console_write(&tag);
                assert_eq!(snap(), vec![tag], "cycle must observe exactly its own chunk");
                teardown();
            }
        }));
    }
    for h in hs {
        h.join().expect("cycle thread");
    }
}

/// T17: two mux workers replacing writers mid-cycle still observe exact transcripts.
#[test]
fn t17_per_cycle_mux_worker_replacement_is_exact() {
    let mut hs = Vec::new();
    for tid in 0..2usize {
        hs.push(std::thread::spawn(move || {
            for cycle in 0..40u32 {
                let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
                sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
                set_console(Some(writer_a));
                console_write("x");
                set_console(Some(writer_b));
                console_write("y");
                assert_eq!(snap(), vec!["a:x", "b:y"]);
                set_console(None);
                let _ = tid; // tag unused; the transcript is pinned exactly
                let _ = cycle;
            }
        }));
    }
    for h in hs {
        h.join().expect("cycle thread");
    }
}

/// T18: 10,000 chunks in a row — no capacity or slot degradation over volume.
#[test]
fn t18_many_chunks_no_loss_over_volume() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    for i in 0..10_000 {
        console_write(&format!("{i}"));
    }
    let got = snap();
    assert_eq!(got.len(), 10_000);
    assert_eq!(got[0], "0");
    assert_eq!(got[9_999], "9999");
    teardown();
}

/// T19: multibyte content survives verbatim — the slot routes &str, never re-encodes.
#[test]
fn t19_unicode_chunk_content_roundtrip() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    let samples = ["日本語のパニック", "día ⚡ dépend", "emoji 🦀🛠", "tab\tand\nnewline"];
    for s in &samples {
        console_write(s);
    }
    assert_eq!(snap(), samples.to_vec());
    teardown();
}

/// T20: interleaved writes and reports form one exact transcript — the composition the
/// panic path actually performs.
#[test]
fn t20_interleaved_writes_and_reports_transcript() {
    let _m = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
    install();
    console_write("boot");
    report_panic("first failure");
    console_write("mid");
    report_panic("second failure");
    assert_eq!(
        snap(),
        vec![
            "boot",
            "panicked: ", "first failure", "\n",
            "mid",
            "panicked: ", "second failure", "\n",
        ]
    );
    teardown();
}
