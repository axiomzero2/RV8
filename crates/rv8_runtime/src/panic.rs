// CEP:FILE: crates/rv8_runtime/src/panic.rs
// CEP:WHAT: The bare-metal panic mechanism: a console callback slot, the panic report
//   path (print the message when a console is registered), and the abort routine.
// CEP:WHY: design.md pins the panic behavior: the handler prints the panic message if a
//   console is available and then either unwinds or aborts; this runtime pins abort as the
//   bare-metal path, because table-based unwinding through .eh_frame is the OS-target
//   alternative owned by the backend ABI (standard 5.9).
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: panic never returns (it aborts); the console callback is best-effort by
//   contract - its failures are ignored and it must not itself panic or allocate.
// CEP:ASSUMES: a bare-metal binary installs report-and-abort as its #[panic_handler]
//   (a library crate cannot define the handler); host tests run under std and exercise
//   the console and message paths only; the console slot is a single writer callback
//   registered once during startup, single-threaded.
// CEP:COST: cold path only; the registered-console check is one atomic load per write.
// CEP:EVIDENCE: unit tests in this module (console registration, report formatting).

use core::sync::atomic::{AtomicUsize, Ordering};

/// CEP:WHAT: The console writer signature: one call per text chunk.
/// CEP:WHY: design.md says the message is printed "if a console is available"; the
///   callback is how a bare-metal target makes a console available.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a failed write is silently dropped by contract.
/// CEP:ASSUMES: the callback is panic-safe, allocation-free, and interrupt-safe enough
///   for a panic path.
/// CEP:COST: callback-defined.
/// CEP:EVIDENCE: test console_callback_receives_text.
pub type ConsoleWrite = fn(&str);

/// CEP:WHAT: The registered console writer, stored as a function pointer in a plain
///   atomic usize (0 = none).
/// CEP:WHY: no_std-safe static storage without dynamic registration machinery.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: function pointers of type fn(&str) round-trip through usize on the
///   supported targets.
/// CEP:COST: one atomic per access.
/// CEP:EVIDENCE: test console_callback_receives_text.
static CONSOLE: AtomicUsize = AtomicUsize::new(0);

/// CEP:WHAT: Registers the console writer, replacing any previous one; None removes it.
/// CEP:WHY: Startup configuration of the panic path.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: called once during startup before any panic, single-threaded.
/// CEP:COST: one atomic store.
/// CEP:EVIDENCE: test console_callback_receives_text.
pub fn set_console(writer: Option<ConsoleWrite>) {
    let raw = match writer {
        Some(f) => f as usize,
        None => 0,
    };
    CONSOLE.store(raw, Ordering::Release);
}

/// CEP:WHAT: Writes one text chunk to the registered console, or drops it when no console
///   is registered.
/// CEP:WHY: The "if a console is available" clause of the design contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; text is silently dropped without a console.
/// CEP:ASSUMES: none
/// CEP:COST: one atomic load plus the callback.
/// CEP:EVIDENCE: test console_callback_receives_text.
pub fn console_write(text: &str) {
    let raw = CONSOLE.load(Ordering::Acquire);
    if raw != 0 {
        let writer = unsafe {
            // SAFETY: raw was stored from a valid fn(&str) by set_console.
            core::mem::transmute::<usize, ConsoleWrite>(raw)
        };
        writer(text);
    }
}

/// CEP:WHAT: Extracts the human-readable message from a panic payload: the &str payload
///   when present, otherwise a fixed placeholder.
/// CEP:WHY: Panic payloads arrive as dyn Any; the panic path needs one deterministic
///   text form.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; unknown payloads map to the placeholder.
/// CEP:ASSUMES: payload downcasting to &str covers the core panic! forms used by this
///   runtime and its collections.
/// CEP:COST: two downcast attempts, cold path.
/// CEP:EVIDENCE: test panic_report_reaches_console.
pub fn payload_text(payload: &(dyn core::any::Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return s;
    }
    "panic payload is not a &str"
}

/// CEP:WHAT: Reports one panic to the console: a fixed prefix followed by the message.
/// CEP:WHY: The print half of the design's panic handler.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; no console means no output, never a panic.
/// CEP:ASSUMES: the console callback does not panic (documented on ConsoleWrite).
/// CEP:COST: cold path.
/// CEP:EVIDENCE: test panic_report_reaches_console.
pub fn report_panic(message: &str) {
    console_write("panicked: ");
    console_write(message);
    console_write("\n");
}

/// CEP:WHAT: The panic handler entry point for bare-metal binaries: reports the panic
///   message through the console (if any) and then aborts; never returns.
/// CEP:WHY: design.md's panic sequence (print, then abort) as one callable function that
///   a binary's `panic_handler` attribute forwards to.
/// CEP:STATUS: complete
/// CEP:FAILURE: aborts unconditionally.
/// CEP:ASSUMES: a binary wires this into its `panic_handler` attribute; unwinding through
///   .eh_frame remains the backend ABI's OS-target alternative (standard 5.9); the message
///   comes from PanicInfo::message().as_str(), which covers every literal panic! form this
///   runtime and its collections use; payload-only panics report a fixed placeholder.
/// CEP:COST: cold path, terminal.
/// CEP:EVIDENCE: the host tests exercise report_panic and the payload helpers directly.
pub fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    match info.message().as_str() {
        Some(text) => report_panic(text),
        None => report_panic("panic payload has no message"),
    }
    abort()
}

/// CEP:WHAT: The terminal abort: under test it is the std abort; on bare metal it is a
///   defined spin (no exit primitive exists without an OS interface).
/// CEP:WHY: The stop half of the design's panic sequence.
/// CEP:STATUS: complete
/// CEP:FAILURE: never returns.
/// CEP:ASSUMES: on bare metal the target's reset or debugger ends the spin; a real halt
///   instruction is a target-specific backend concern.
/// CEP:COST: terminal.
/// CEP:EVIDENCE: this module's tests rely on the test path only.
pub fn abort() -> ! {
    #[cfg(test)]
    {
        std::process::abort();
    }
    #[cfg(not(test))]
    {
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// CEP:WHAT: The test sink pointer: the currently registered callback appends chunks
    ///   to the leaked Vec this points at.
    /// CEP:WHY: Test fixture plumbing for a fn(&str) callback, which cannot capture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: every test that touches the process-global console locks
    ///   CONSOLE_MUX first; lib tests run on parallel threads.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    static SINK: AtomicUsize = AtomicUsize::new(0);

    /// CEP:WHAT: Serializes every console-using test in this module.
    /// CEP:WHY: `set_console` registers a process-global writer, and cargo runs
    ///   lib tests on parallel threads: without this mux, one test's
    ///   install_sink could clobber the console between another test's writes
    ///   (observed as a ~10-20% flake: `console_callback_receives_text` saw
    ///   ["hello"] instead of ["hello", "world"]).  The mux makes the
    ///   install/write/assert/remove sequence atomic per test without weakening
    ///   any assertion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: poisoning is deliberately ignored (a failed assert must not
    ///   cascade into the sibling test's lock).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module; the flake it fixes is documented in the ticket.
    static CONSOLE_MUX: Mutex<()> = Mutex::new(());

    /// CEP:WHAT: Installs the capturing console and returns the shared sink.
    /// CEP:WHY: Shared fixture for the two panic-path tests.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn install_sink() -> &'static Mutex<std::vec::Vec<String>> {
        let sink: &'static Mutex<std::vec::Vec<String>> =
            Box::leak(Box::new(Mutex::new(std::vec::Vec::new())));
        SINK.store(sink as *const _ as usize, Ordering::Release);
        fn writer(text: &str) {
            let ptr = SINK.load(Ordering::Acquire) as *const Mutex<std::vec::Vec<String>>;
            unsafe {
                // SAFETY: SINK was set from a leaked &'static Mutex by install_sink.
                (*ptr).lock().expect("sink lock").push(text.to_string());
            }
        }
        set_console(Some(writer));
        sink
    }

    /// CEP:WHAT: A registered console callback receives every chunk in order, and nothing
    ///   arrives once the console is removed.
    /// CEP:WHY: The registration and write half of the panic path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn console_callback_receives_text() {
        let _mux = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
        let sink = install_sink();
        console_write("hello");
        console_write("world");
        assert_eq!(
            sink.lock().expect("sink lock").clone(),
            std::vec!["hello", "world"]
        );
        set_console(None);
        console_write("dropped");
        assert_eq!(sink.lock().expect("sink lock").len(), 2);
    }

    /// CEP:WHAT: report_panic emits the fixed prefix, the message, and the newline, and
    ///   payload_text maps unknown payloads to the placeholder.
    /// CEP:WHY: The report half of the panic path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn panic_report_reaches_console() {
        let _mux = CONSOLE_MUX.lock().unwrap_or_else(|e| e.into_inner());
        let sink = install_sink();
        report_panic("unwrap on None");
        assert_eq!(
            sink.lock().expect("sink lock").clone(),
            std::vec!["panicked: ", "unwrap on None", "\n"]
        );
        assert_eq!(payload_text(&"boom"), "boom");
        assert_eq!(payload_text(&42usize), "panic payload is not a &str");
        set_console(None);
    }
}
