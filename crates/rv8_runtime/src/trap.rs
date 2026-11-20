// CEP:FILE: crates/rv8_runtime/src/trap.rs
// CEP:WHAT: The deopt trap interception path (design.md "Region-Based Partial
//   Deoptimization" 1.3, RV8-141): a SIGILL handler that catches a guard's ud2 trap,
//   reads the faulting instruction pointer and the whole machine register file out of
//   the signal context, reconstructs the region's deoptimization state exactly - every
//   live value from its allocated register (translated through the pinned
//   machir-PhysReg -> gregs-index table) or its RSP-relative spill slot - records the
//   captured values into the caller's fixed buffer, and redirects execution to the
//   deopt exit stub by rewriting RIP (the stub's leave/ret returns to the AOT caller
//   with the reconstructed return value in RAX).
// CEP:WHY: STANDARDS 5.13 acceptance clause 2: "a deopt reconstructs the interpreter
//   state exactly". The state bitstream decoded here is the pinned wire format of
//   rv8_backend::deopt::encode_state (the encode RelocKind-mirror precedent): a u32
//   region, a u16 count, then per entry a u32 value id, a u8 kind, and a payload of
//   u16 reg or i32 offset.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete for the trap-capture-reconstruct-redirect path; the region-
//   granular resume inside AOT code (the OSR side, design 2) remains per RV8-141.
// CEP:FAILURE: a trap outside the registered code range, a malformed state bitstream,
//   or a location outside the 16-register machine file refuses the trap: the handler
//   resets SIGILL to SIG_DFL and returns, so the kernel re-executes the ud2 under the
//   default action and the process dies loudly instead of continuing on reconstructed
//   garbage. Library code never panics.
// CEP:ASSUMES: linux x86-64 (glibc ucontext_t gregs layout: R8=0..R15=7, RDI=8, RSI=9,
//   RBP=10, RBX=11, RDX=12, RAX=13, RCX=14, RSP=15, RIP=16); the registered entry is
//   quiesced and outlives every trap (the tier is single-threaded per jitmem's
//   contract); the exit stub is the function's own position-independent epilogue
//   (add rsp / pops / ret) copied by the caller, so the handler's RIP rewrite lands
//   the interrupted frame back on the AOT caller without touching RSP or RBP.
// CEP:COST: one trap is a kernel round trip plus O(state entries) reconstruction; the
//   installed handler adds nothing to non-trapping execution.
// CEP:EVIDENCE: the unit tests here drive the handler directly over a fabricated
//   signal context (exact capture, RAX/RIP rewrite, refuse paths, register mapping)
//   and tests/jit_deopt.rs executes a real ud2 trap end to end in-process.
// CEP:SECURITY: the handler allocates nothing, takes no locks, and touches only the
//   registered entry's fixed buffers and the signal context (async-signal-safe);
//   mmap-like syscalls are declared locally with zero external crates, mirroring
//   jitmem; an unregistered or malformed trap fails closed through SIG_DFL.

use core::sync::atomic::{AtomicPtr, Ordering};

/// CEP:WHAT: Maximum live entries the handler reconstructs (pinned mirror of
///   rv8_backend::deopt::MAX_DEOPT_STATE_ENTRIES).
/// CEP:WHY: The capture buffer is fixed-sized so the handler never allocates.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the backend rejects states with more entries at encode time.
/// CEP:COST: constant.
/// CEP:EVIDENCE: the handler refuses a larger decoded count.
pub const MAX_DEOPT_VALUES: usize = 10;

/// CEP:WHAT: Byte budget of the state bitstream (pinned mirror of
///   rv8_backend::deopt::DEOPT_STATE_MAX_BYTES).
/// CEP:WHY: One storage constant on each side of the wire contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: mirrors the backend constant exactly.
/// CEP:COST: constant.
/// CEP:EVIDENCE: TrapEntry's fixed state array.
pub const DEOPT_STATE_MAX_BYTES: usize = 100;

/// CEP:WHAT: The intercepted signal (x86-64 illegal instruction) and the SA_SIGINFO
///   flag of the glibc sigaction interface.
/// CEP:WHY: A guard's ud2 raises SIGILL; the handler needs the third-argument form to
///   reach the machine context.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: linux numbering.
/// CEP:COST: constant.
/// CEP:EVIDENCE: install registers exactly these.
pub const SIGILL: i32 = 4;
/// CEP:WHAT: SA_SIGINFO flag value.
/// CEP:WHY: See SIGILL.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: linux numbering.
/// CEP:COST: constant.
/// CEP:EVIDENCE: install sets it.
pub const SA_SIGINFO: i32 = 4;

/// CEP:WHAT: One reconstructed value: the deopt state's value id and the raw 64-bit
///   content captured from its machine location.
/// CEP:WHY: The caller's buffer consumes exactly this pair per live value.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: repr(C) for the signal-handler raw writes.
/// CEP:COST: 16 bytes per entry.
/// CEP:EVIDENCE: capture unit test asserts exact id/raw pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CapturedValue {
    /// The deopt state's value id.
    pub id: u32,
    /// The raw 64-bit content at the value's machine location.
    pub raw: i64,
}

/// CEP:WHAT: One registered deopt context: the code range the handler owns, the state
///   bitstream to reconstruct, the exit stub to redirect into, the return value id for
///   RAX, and the caller's fixed capture buffer.
/// CEP:WHY: Design 1.3 steps 2-5: the handler needs the metadata table (here: one
///   quiesced entry, the tier's single deopt context) plus the resume target.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the entry outlives every trap and the buffers are quiesced at
///   registration (single-threaded tier).
/// CEP:COST: fixed-size POD.
/// CEP:EVIDENCE: the unit tests register synthetic entries and drive the handler.
#[derive(Debug)]
#[repr(C)]
pub struct TrapEntry {
    /// First byte address of the function's code (the module's entry address).
    pub code_start: usize,
    /// Length of the function's code in bytes.
    pub code_len: usize,
    /// The state bitstream (rv8_backend::deopt wire format).
    pub state: [u8; DEOPT_STATE_MAX_BYTES],
    /// Meaningful length of `state`.
    pub state_len: usize,
    /// Address of the deopt exit stub execution resumes into.
    pub exit_stub: usize,
    /// Value id whose captured raw content becomes RAX at the exit (NONE when the
    /// function returns nothing).
    pub ret_value: u32,
    /// Capture buffer (caller-owned, quiesced).
    pub out: *mut CapturedValue,
    /// Capture buffer capacity.
    pub out_cap: usize,
    /// Where the handler stores the captured count (caller-owned).
    pub out_len: *mut usize,
}

/// CEP:WHAT: Failure of installing the SIGILL handler.
/// CEP:WHY: sigaction can fail on signal-mask edge cases; the caller must know.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this IS the failure surface.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: install returns it on a nonzero sigaction result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapError {
    /// The kernel refused the sigaction call; payload is the errno.
    InstallFailed(i32),
}

/// CEP:WHAT: The glibc ucontext gregs index for each machir PhysReg number (0 = RAX ..
///   15 = R15): RAX=13, RCX=14, RDX=12, RBX=11, RSP=15, RBP=10, RSI=9, RDI=8,
///   R8..R15 = 0..7.
/// CEP:WHY: The deopt locations carry machir register numbers; the signal context
///   carries glibc's gregs ordering. One pinned translation table keeps the two
///   conventions from drifting.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc x86-64 gregs layout (see the module CEP:ASSUMES).
/// CEP:COST: constant table.
/// CEP:EVIDENCE: gregs_mapping unit test pins every entry.
const GREGS_OF_MACHIR: [usize; 16] = [
    13, // machir 0: RAX
    14, // machir 1: RCX
    12, // machir 2: RDX
    11, // machir 3: RBX
    15, // machir 4: RSP
    10, // machir 5: RBP
    9,  // machir 6: RSI
    8,  // machir 7: RDI
    0, 1, 2, 3, 4, 5, 6, 7, // machir 8..15: R8..R15
];

/// CEP:WHAT: glibc x86-64 machine context: the 23 gregs slots plus the fpregs pointer
///   and padding, laid out so `gregs` starts at ucontext offset 40.
/// CEP:WHY: The handler reads and rewrites gregs through this shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc layout; uc_flags + uc_link + uc_stack (24 bytes) precede it.
/// CEP:COST: POD.
/// CEP:EVIDENCE: the fabricated-context unit tests place gregs at offset 40.
#[repr(C)]
pub struct MachineContext {
    /// The 23 general register slots (glibc gregs ordering).
    pub gregs: [i64; 23],
    /// ucontext_t's fpregs pointer (unused, kept for layout).
    pub fpregs: usize,
    /// glibc's trailing padding.
    pub pad: [usize; 8],
}

/// CEP:WHAT: glibc x86-64 ucontext_t prefix: flags, link, signal stack, then the
///   machine context.
/// CEP:WHY: The handler's third argument; only gregs are consumed or rewritten.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: uc_mcontext at byte offset 40 (8 + 8 + 24).
/// CEP:COST: POD.
/// CEP:EVIDENCE: the unit tests assert the gregs offset.
#[repr(C)]
pub struct UContext {
    /// uc_flags.
    pub flags: usize,
    /// uc_link.
    pub link: usize,
    /// uc_stack (ss_sp, ss_flags + pad, ss_size).
    pub stack: [u8; 24],
    /// The machine context.
    pub mcontext: MachineContext,
}

/// CEP:WHAT: Opaque siginfo_t (128 bytes on linux); the handler never reads it.
/// CEP:WHY: The SA_SIGINFO handler signature carries it through.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: size covers the kernel's siginfo.
/// CEP:COST: POD.
/// CEP:EVIDENCE: the handler signature.
#[repr(C)]
pub struct SigInfo {
    /// Opaque bytes.
    pub _bytes: [u8; 128],
}

/// CEP:WHAT: glibc struct sigaction layout (handler, 128-byte mask, flags, restorer).
/// CEP:WHY: Installing the handler through the libc symbol (not a raw syscall) needs
///   this exact shape; glibc supplies its restorer trampoline itself.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc x86-64 ordering (handler, mask, flags, restorer).
/// CEP:COST: POD.
/// CEP:EVIDENCE: install/uninstall round-trip.
#[repr(C)]
struct SigActionRaw {
    /// Handler (or SIG_DFL = 0).
    sa_sigaction: usize,
    /// 128-byte signal mask.
    sa_mask: [u64; 16],
    /// Flags.
    sa_flags: i32,
    /// glibc restorer (libc fills it; zero here).
    sa_restorer: usize,
}

extern "C" {
    fn sigaction(signum: i32, act: *const SigActionRaw, old: *mut SigActionRaw) -> i32;
}

/// CEP:WHAT: The single registered deopt context of this tier.
/// CEP:WHY: One pointer load is the handler's entire lookup; the tier is
///   single-threaded (jitmem contract) so one slot is the honest scope.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: entries are quiesced and outlive their registration window.
/// CEP:COST: one atomic load per trap.
/// CEP:EVIDENCE: register/install/uninstall.
static TRAP_ENTRY: AtomicPtr<TrapEntry> = AtomicPtr::new(core::ptr::null_mut());

/// CEP:WHAT: glibc gregs indices used by the handler: RAX, RBP, RSP, RIP.
/// CEP:WHY: Named so the rewrite sites read as ABI facts, not magic numbers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc ordering.
/// CEP:COST: constants.
/// CEP:EVIDENCE: pinned by gregs_mapping.
const GREG_RAX: usize = 13;
/// CEP:WHAT: glibc gregs index of RSP.
/// CEP:WHY: See GREG_RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc ordering.
/// CEP:COST: constant.
/// CEP:EVIDENCE: pinned by gregs_mapping.
const GREG_RSP: usize = 15;
/// CEP:WHAT: glibc gregs index of RIP.
/// CEP:WHY: See GREG_RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: glibc ordering.
/// CEP:COST: constant.
/// CEP:EVIDENCE: pinned by gregs_mapping.
const GREG_RIP: usize = 16;

/// CEP:WHAT: Installs the SIGILL handler (SA_SIGINFO) and clears the registry.
/// CEP:WHY: The trap path's gate; idempotent by re-registration semantics (the old
///   action is overwritten).
/// CEP:STATUS: complete
/// CEP:FAILURE: TrapError::InstallFailed with the errno on a refused sigaction.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: one sigaction call.
/// CEP:EVIDENCE: the integration witness installs it before trapping.
pub fn install() -> Result<(), TrapError> {
    TRAP_ENTRY.store(core::ptr::null_mut(), Ordering::Relaxed);
    let act = SigActionRaw {
        sa_sigaction: trap_handler as *const () as usize,
        sa_mask: [0; 16],
        sa_flags: SA_SIGINFO,
        sa_restorer: 0,
    };
    let rc = unsafe { sigaction(SIGILL, &act, core::ptr::null_mut()) };
    if rc != 0 {
        return Err(TrapError::InstallFailed(rc));
    }
    Ok(())
}

/// CEP:WHAT: Restores the previous SIGILL action captured at install time by resetting
///   to the default disposition and clearing the registry.
/// CEP:WHY: Tests and the runtime teardown must not leave the handler installed past
///   the entry's lifetime.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (a refused sigaction is swallowed: the registry is cleared either
///   way, so a later trap fails closed into the default action).
/// CEP:ASSUMES: none
/// CEP:COST: one sigaction call.
/// CEP:EVIDENCE: uninstall_restores_default unit test.
pub fn uninstall() {
    TRAP_ENTRY.store(core::ptr::null_mut(), Ordering::Relaxed);
    let dfl = SigActionRaw {
        sa_sigaction: 0,
        sa_mask: [0; 16],
        sa_flags: 0,
        sa_restorer: 0,
    };
    unsafe {
        let _ = sigaction(SIGILL, &dfl, core::ptr::null_mut());
    }
}

/// CEP:WHAT: Registers one deopt context (quiesced, 'static: it outlives every trap).
/// CEP:WHY: The handler's lookup is exactly one pointer load of this entry.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the buffers behind `out`/`out_len` are quiesced; single-threaded tier.
/// CEP:COST: one atomic store.
/// CEP:EVIDENCE: the integration witness registers its entry before trapping.
pub fn register(entry: &'static TrapEntry) {
    TRAP_ENTRY.store(entry as *const TrapEntry as *mut TrapEntry, Ordering::Relaxed);
}

/// CEP:WHAT: Refuses the trap: reset SIGILL to SIG_DFL so the ud2 re-executes under
///   the default action and the process dies loudly.
/// CEP:WHY: Design 1.3's fail-closed rule - a trap the table cannot answer must never
///   be papered over with garbage state; and a plain return would re-enter this
///   handler forever (the ud2 re-executes), so the default action is the only exit.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: called from the handler only.
/// CEP:COST: one sigaction call.
/// CEP:EVIDENCE: refuse-path unit tests.
fn refuse() {
    let dfl = SigActionRaw {
        sa_sigaction: 0,
        sa_mask: [0; 16],
        sa_flags: 0,
        sa_restorer: 0,
    };
    unsafe {
        let _ = sigaction(SIGILL, &dfl, core::ptr::null_mut());
    }
}

/// CEP:WHAT: The SIGILL handler: locate the entry by RIP, decode the state bitstream
///   (no allocation), capture every live value from its register or spill slot,
///   record them, select RAX, and rewrite RIP to the exit stub.
/// CEP:WHY: Design 1.3 steps 2-5 - the whole reconstruction happens in the handler so
///   execution resumes exactly once, at the deopt exit.
/// CEP:STATUS: complete
/// CEP:FAILURE: any surprise (unknown RIP, malformed state, register number out of the
///   machine file, capture buffer overrun) routes to refuse().
/// CEP:ASSUMES: called by the kernel with a valid ucontext; the entry is quiesced.
/// CEP:COST: O(state entries); no allocation.
/// CEP:EVIDENCE: the direct-call unit tests and the real-trap integration witness.
unsafe extern "C" fn trap_handler(_sig: i32, _info: *mut SigInfo, uctx: *mut UContext) {
    let entry = TRAP_ENTRY.load(Ordering::Relaxed);
    if entry.is_null() || uctx.is_null() {
        refuse();
        return;
    }
    let entry = &*entry;
    let gregs = &uctx.as_ref().expect("checked above").mcontext.gregs;
    let rip = gregs[GREG_RIP] as usize;
    if rip < entry.code_start || rip >= entry.code_start + entry.code_len {
        refuse();
        return;
    }
    // Decode the wire format in place (no allocation, fixed bounds).
    let bytes = &entry.state[..entry.state_len];
    if bytes.len() < 6 {
        refuse();
        return;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count > MAX_DEOPT_VALUES {
        refuse();
        return;
    }
    let rsp = gregs[GREG_RSP] as usize;
    let mut cur = 6usize;
    let mut captured = 0usize;
    while captured < count {
        if cur + 5 > bytes.len() {
            refuse();
            return;
        }
        let id = u32::from_le_bytes([
            bytes[cur],
            bytes[cur + 1],
            bytes[cur + 2],
            bytes[cur + 3],
        ]);
        let kind = bytes[cur + 4];
        cur += 5;
        let raw = match kind {
            0 => {
                if cur + 2 > bytes.len() {
                    refuse();
                    return;
                }
                let r = u16::from_le_bytes([bytes[cur], bytes[cur + 1]]) as usize;
                cur += 2;
                if r >= 16 {
                    refuse();
                    return;
                }
                gregs[GREGS_OF_MACHIR[r]]
            }
            1 => {
                if cur + 4 > bytes.len() {
                    refuse();
                    return;
                }
                let off = i32::from_le_bytes([
                    bytes[cur],
                    bytes[cur + 1],
                    bytes[cur + 2],
                    bytes[cur + 3],
                ]);
                cur += 4;
                let addr = (rsp as i64 + off as i64) as *const i64;
                *addr
            }
            _ => {
                refuse();
                return;
            }
        };
        if captured >= entry.out_cap || entry.out.is_null() {
            refuse();
            return;
        }
        // SAFETY: the caller's fixed buffer, quiesced and sized out_cap at least
        // count; the write is the handler's sole access.
        *entry.out.add(captured) = CapturedValue { id, raw };
        captured += 1;
    }
    if entry.out_len.is_null() {
        refuse();
        return;
    }
    // SAFETY: caller-owned count slot, quiesced.
    *entry.out_len = captured;
    // The function's return value rides in RAX at the exit stub.
    let mut rax = gregs[GREG_RAX];
    for i in 0..captured {
        let cv = &*entry.out.add(i);
        if cv.id == entry.ret_value {
            rax = cv.raw;
            break;
        }
    }
    // SAFETY: the ucontext belongs to this thread's interrupted frame; rewriting the
    // machine context is the documented resume mechanism (design 1.3 step 5).
    let ctx = &mut *uctx;
    ctx.mcontext.gregs[GREG_RAX] = rax;
    ctx.mcontext.gregs[GREG_RIP] = entry.exit_stub as i64;
}

#[cfg(all(test, target_arch = "x86_64", target_os = "linux"))]
mod tests {
    use super::*;
    use core::mem::size_of;

    /// CEP:WHAT: The machir -> gregs translation table pins every entry against the
    ///   documented ABI facts.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn gregs_mapping_table() {
        let want = [13usize, 14, 12, 11, 15, 10, 9, 8, 0, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(GREGS_OF_MACHIR, want);
        assert_eq!(GREG_RAX, 13);
        assert_eq!(GREG_RSP, 15);
        assert_eq!(GREG_RIP, 16);
    }

    /// CEP:WHAT: gregs sits at ucontext offset 40 and MachineContext keeps the glibc
    ///   23-slot shape.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn ucontext_layout() {
        let base = UContext {
            flags: 0,
            link: 0,
            stack: [0; 24],
            mcontext: MachineContext {
                gregs: [0; 23],
                fpregs: 0,
                pad: [0; 8],
            },
        };
        let base_addr = &base as *const UContext as usize;
        let gregs_addr = &base.mcontext.gregs as *const [i64; 23] as usize;
        assert_eq!(gregs_addr - base_addr, 40);
        assert_eq!(size_of::<MachineContext>(), 23 * 8 + 8 + 64);
    }

    /// CEP:WHAT: Drives the handler directly over a fabricated signal context: a
    ///   register-resident value (RAX) and a spill-slot value are captured exactly,
    ///   RAX becomes the return value's content, and RIP points at the exit stub.
    /// CEP:WHY: The STANDARDS 5.13 "reconstructs exactly" contract, unit-scale.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn handler_captures_exact_state() {
        // Wire: region 5, two entries - value 0 in machir RAX(0), value 3 at rsp-8.
        let mut state = [0u8; DEOPT_STATE_MAX_BYTES];
        state[0..4].copy_from_slice(&5u32.to_le_bytes());
        state[4..6].copy_from_slice(&2u16.to_le_bytes());
        // entry 0: id 0, kind 0, machir reg 0 (RAX).
        state[6..10].copy_from_slice(&0u32.to_le_bytes());
        state[10] = 0;
        state[11..13].copy_from_slice(&0u16.to_le_bytes());
        // entry 1: id 3, kind 1, offset -8.
        state[13..17].copy_from_slice(&3u32.to_le_bytes());
        state[17] = 1;
        state[18..22].copy_from_slice(&(-8i32).to_le_bytes());
        // Memory: a real mapped backing array stands in for the frame; rsp points 32
        // slots in, and the spill slot at rsp-8 holds 77.
        let mut backing = [0i64; 64];
        let frame = backing.as_mut_ptr() as usize + 8 * 32;
        let slot_addr = (frame - 8) as *mut i64;
        unsafe { slot_addr.write_volatile(77) };
        // Fabricated context: RAX = 13 (the register-resident value), RSP = 0x5000.
        let mut uctx = UContext {
            flags: 0,
            link: 0,
            stack: [0; 24],
            mcontext: MachineContext {
                gregs: [0; 23],
                fpregs: 0,
                pad: [0; 8],
            },
        };
        uctx.mcontext.gregs[GREG_RAX] = 13;
        uctx.mcontext.gregs[GREG_RSP] = frame as i64;
        uctx.mcontext.gregs[GREG_RIP] = 0x4000; // inside the fake code range
        let mut out = [CapturedValue { id: 0, raw: 0 }; MAX_DEOPT_VALUES];
        let mut out_len = 0usize;
        let entry = std::boxed::Box::leak(std::boxed::Box::new(TrapEntry {
            code_start: 0x4000,
            code_len: 0x100,
            state,
            state_len: 22,
            exit_stub: 0x9000,
            ret_value: 3,
            out: out.as_mut_ptr(),
            out_cap: MAX_DEOPT_VALUES,
            out_len: &mut out_len,
        }));
        register(entry);
        // SAFETY: fabricated context and entry; the handler is a plain fn here.
        unsafe { trap_handler(SIGILL, core::ptr::null_mut(), &mut uctx) };
        assert_eq!(out_len, 2);
        assert_eq!(out[0], CapturedValue { id: 0, raw: 13 });
        assert_eq!(out[1], CapturedValue { id: 3, raw: 77 });
        assert_eq!(uctx.mcontext.gregs[GREG_RAX], 77, "RAX becomes the return value");
        assert_eq!(uctx.mcontext.gregs[GREG_RIP], 0x9000, "RIP redirected to the stub");
        uninstall();
    }

    /// CEP:WHAT: A trap outside the registered range and a malformed state both refuse:
    ///   the capture buffer stays untouched and RIP is left alone.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn handler_refuses_fail_closed() {
        let mut state = [0u8; DEOPT_STATE_MAX_BYTES];
        state[0..4].copy_from_slice(&0u32.to_le_bytes());
        state[4..6].copy_from_slice(&1u16.to_le_bytes());
        state[6..10].copy_from_slice(&0u32.to_le_bytes());
        state[10] = 0; // kind register
        state[11..13].copy_from_slice(&0u16.to_le_bytes());
        let mut out = [CapturedValue { id: 0, raw: 0 }; MAX_DEOPT_VALUES];
        let mut out_len = 0usize;
        let mut uctx = UContext {
            flags: 0,
            link: 0,
            stack: [0; 24],
            mcontext: MachineContext {
                gregs: [0; 23],
                fpregs: 0,
                pad: [0; 8],
            },
        };
        uctx.mcontext.gregs[GREG_RIP] = 0x4FF; // one byte below the range
        // Unknown-RIP refusal.
        let entry = std::boxed::Box::leak(std::boxed::Box::new(TrapEntry {
            code_start: 0x500,
            code_len: 0x100,
            state,
            state_len: 13,
            exit_stub: 0x9000,
            ret_value: 0,
            out: out.as_mut_ptr(),
            out_cap: MAX_DEOPT_VALUES,
            out_len: &mut out_len,
        }));
        register(entry);
        // SAFETY: fabricated context and entry.
        unsafe { trap_handler(SIGILL, core::ptr::null_mut(), &mut uctx) };
        assert_eq!(out_len, 0, "nothing captured on refusal");
        assert_eq!(uctx.mcontext.gregs[GREG_RIP], 0x4FF, "RIP untouched on refusal");
        // Malformed state (truncated header) refusal: reset the buffer, put RIP inside.
        out_len = 0;
        let mut bad = [0u8; DEOPT_STATE_MAX_BYTES];
        bad[4] = 0xFF; // count high byte: 0xFFxx over the budget
        bad[5] = 0xFF;
        let bad_entry = std::boxed::Box::leak(std::boxed::Box::new(TrapEntry {
            code_start: 0x500,
            code_len: 0x100,
            state: bad,
            state_len: 6,
            exit_stub: 0x9000,
            ret_value: 0,
            out: out.as_mut_ptr(),
            out_cap: MAX_DEOPT_VALUES,
            out_len: &mut out_len,
        }));
        register(bad_entry);
        uctx.mcontext.gregs[GREG_RIP] = 0x600;
        // SAFETY: fabricated context and entry.
        unsafe { trap_handler(SIGILL, core::ptr::null_mut(), &mut uctx) };
        assert_eq!(out_len, 0, "nothing captured on a malformed state");
        assert_eq!(uctx.mcontext.gregs[GREG_RIP], 0x600, "RIP untouched on refusal");
        uninstall();
    }

    /// CEP:WHAT: install() succeeds and uninstall() clears the registry (the handler
    ///   then refuses by the null-entry path).
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn install_uninstall_round_trip() {
        install().expect("install");
        assert!(TRAP_ENTRY.load(Ordering::Relaxed).is_null(), "install clears the registry");
        uninstall();
        assert!(TRAP_ENTRY.load(Ordering::Relaxed).is_null());
    }
}
