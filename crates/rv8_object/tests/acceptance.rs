// CEP:FILE: crates/rv8_object/tests/acceptance.rs
// CEP:WHAT: Acceptance suite for rv8_object (STANDARDS 5.5): the end-to-end link-and-run
//   heartbeat (exit code 13), byte-determinism of both writers, negative resolution and
//   validation tests, string-table suffix sharing, and section-GC behavior.
// CEP:WHY: These tests are the project heartbeat: they prove the universal model, the
//   ELF64 writers, and the static linker agree on bytes a real kernel can execute.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: linux x86_64 for the exec test (other hosts skip with a clear message);
//   /tmp accepts executables.
// CEP:COST: test-only; links are small and deterministic.
// CEP:EVIDENCE: this file is the evidence; every required behavior has one test below.

use rv8_object::elf::write_elf64_relocatable;
use rv8_object::elf::StringTable;
use rv8_object::link::{link_executable, plan_executable, synthesize_start, DEFAULT_BASE_VADDR};
use rv8_object::model::{ObjectFile, Reloc, RelocKind, Section, SectionType, FLAG_ALLOC, FLAG_EXEC};
use rv8_object::ObjectError;

/// Builds the `add` object: `.text` = `8d 04 37` (lea eax, [rdi+rsi*1]) + `c3` (ret),
/// SysV AMD64: adds edi+esi into eax. Global symbol `add` at offset 0, size 4.
fn add_object() -> ObjectFile {
    let mut o = ObjectFile::new("add.o");
    let t = o
        .add_text_section(".text", 16, vec![0x8d, 0x04, 0x37, 0xc3])
        .expect("add .text");
    o.add_symbol("add", true, true, Some(t), 0, 4).expect("add symbol");
    o
}

/// Builds the `main` object: `.text` = `bf 06 00 00 00` (mov edi, 6),
/// `be 07 00 00 00` (mov esi, 7), `e8 rel32` (call add; the rel32 FIELD sits at
/// offset 11, which is the PcRel32 relocation offset; addend -4 gives
/// S + A - P = add - (0x0b + 4) relative to the section base, i.e. a call from
/// instruction offset 10 spanning bytes 10..15), `c3` (ret). Global `main`, size 21.
fn main_object() -> ObjectFile {
    let mut o = ObjectFile::new("main.o");
    let bytes = vec![
        0xbf, 0x06, 0x00, 0x00, 0x00, // mov edi, 6
        0xbe, 0x07, 0x00, 0x00, 0x00, // mov esi, 7
        0xe8, 0x00, 0x00, 0x00, 0x00, // call rel32 -> add (relocated at offset 11)
        0xc3, // ret
    ];
    let t = o.add_text_section(".text", 16, bytes).expect("main .text");
    o.add_symbol("main", true, true, Some(t), 0, 21).expect("main symbol");
    o.add_reloc(t, 11, RelocKind::PcRel32, "add", -4).expect("call reloc");
    o
}

/// CEP:WHAT: The heartbeat: link add/main plus the _start shim and EXECUTE the result;
///   the process must exit with 6 + 7 = 13.
/// CEP:WHY: STANDARDS 5.5 acceptance.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on link error, write error, or unexpected exit code (test-only).
/// CEP:ASSUMES: linux x86_64; temp dir executable.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn end_to_end_link_and_run_exits_13() {
    let objs = [add_object(), main_object(), synthesize_start("main")];
    let image = link_executable(&objs, "_start", 0x0040_0000, true).expect("link succeeds");
    assert_eq!(&image[0..4], b"\x7fELF");
    let path = std::env::temp_dir().join("rv8_e2e_test");
    std::fs::write(&path, &image).expect("write image");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("set exec bits");
    }
    let status = std::process::Command::new(&path)
        .status()
        .expect("execute linked image");
    assert_eq!(status.code(), Some(13), "expected exit code 6 + 7 = 13");
}

/// CEP:WHAT: Non-x86_64-linux hosts skip execution but still verify the link succeeds.
/// CEP:WHY: Keeps the suite honest on other hosts.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on link error (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn end_to_end_link_and_run_exits_13() {
    println!("skipped: end-to-end exec requires linux x86_64; linking is still verified");
    let objs = [add_object(), main_object(), synthesize_start("main")];
    assert!(link_executable(&objs, "_start", 0x0040_0000, true).is_ok());
}

/// CEP:WHAT: Both writers are byte-deterministic: relocatable emission twice and
///   executable emission twice produce identical Vec<u8>s.
/// CEP:WHY: STANDARDS 5.5: two runs over the same input emit identical bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn determinism_both_writers() {
    let objs = [add_object(), main_object(), synthesize_start("main")];
    let a = write_elf64_relocatable(&objs[0]).unwrap();
    let b = write_elf64_relocatable(&objs[0]).unwrap();
    assert_eq!(a, b, "relocatable writer must be deterministic");

    let x = link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
    let y = link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
    assert_eq!(x, y, "executable writer must be deterministic");

    let p1 = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, false).unwrap();
    let p2 = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, false).unwrap();
    assert_eq!(p1, p2, "planner must be deterministic");
}

/// CEP:WHAT: Duplicate strong global definitions across objects are rejected.
/// CEP:WHY: Required negative test (resolution rule).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn duplicate_strong_symbol_rejected() {
    let mut a = ObjectFile::new("a.o");
    let ta = a.add_text_section(".text", 1, vec![0xc3]).unwrap();
    a.add_symbol("clash", true, true, Some(ta), 0, 1).unwrap();
    let mut b = ObjectFile::new("b.o");
    let tb = b.add_text_section(".text", 1, vec![0xc3]).unwrap();
    b.add_symbol("clash", true, true, Some(tb), 0, 1).unwrap();
    let objs = [a, b, synthesize_start("clash")];
    assert_eq!(
        link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true),
        Err(ObjectError::DuplicateStrongSymbol { name: "clash".into() })
    );
}

/// CEP:WHAT: An undefined reference with gc_sections=false is rejected.
/// CEP:WHY: Required negative test.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn undefined_symbol_rejected_without_gc() {
    let mut o = ObjectFile::new("missing.o");
    let t = o
        .add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0, 0xc3])
        .unwrap();
    o.add_symbol("main", true, true, Some(t), 0, 6).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "missing_fn", -4).unwrap();
    let objs = [o, synthesize_start("main")];
    assert_eq!(
        link_executable(&objs, "_start", DEFAULT_BASE_VADDR, false),
        Err(ObjectError::UndefinedSymbol { name: "missing_fn".into() })
    );
}

/// CEP:WHAT: A section whose alignment is not a power of two fails model validation and
///   therefore both writers.
/// CEP:WHY: Required negative test (validation rule).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn non_power_of_two_align_rejected() {
    let bad = Section {
        name: ".misaligned".into(),
        ty: SectionType::Progbits,
        flags: FLAG_ALLOC | FLAG_EXEC,
        align: 3,
        bytes: vec![0x90],
        relocs: Vec::new(),
    };
    let mut o = ObjectFile::new("misaligned.o");
    o.sections.push(bad);
    assert_eq!(
        o.validate(),
        Err(ObjectError::InvalidAlignment { section: ".misaligned".into(), align: 3 })
    );
    assert_eq!(
        write_elf64_relocatable(&o),
        Err(ObjectError::InvalidAlignment { section: ".misaligned".into(), align: 3 })
    );
    assert!(link_executable(&[o], "_start", DEFAULT_BASE_VADDR, true).is_err());
}

/// CEP:WHAT: A PcRel32 relocation whose computed value exceeds i32 fails closed.
/// CEP:WHY: Required negative test (range check).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: the addend drives the range violation without huge sections.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn pcrel_out_of_i32_range_rejected() {
    let mut o = ObjectFile::new("far.o");
    let t = o
        .add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0, 0xc3])
        .unwrap();
    o.add_symbol("main", true, true, Some(t), 0, 6).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "main", 1 << 40).unwrap();
    let objs = [o, synthesize_start("main")];
    match link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true) {
        Err(ObjectError::RelocOutOfRange { kind, .. }) => assert_eq!(kind, "PcRel32"),
        other => panic!("expected RelocOutOfRange, got {other:?}"),
    }
}

/// CEP:WHAT: String-table suffix sharing: "main", "add_main", exact repeats, and suffix
///   strings share storage.
/// CEP:WHY: Required test (STANDARDS 5.5 names suffix sharing).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test (mirrored in rv8_object::elf::tests).
#[test]
fn string_table_suffix_sharing() {
    let mut st = StringTable::new();
    assert_eq!(st.add("main").unwrap(), 1);
    let add_main = st.add("add_main").unwrap();
    assert_eq!(add_main, 6);
    assert_eq!(st.add("main").unwrap(), 1);
    assert_eq!(st.add("ain").unwrap(), 2);
    assert_eq!(st.add("_main").unwrap(), add_main + 3);
    assert_eq!(st.as_bytes(), b"\0main\0add_main\0");
}

/// CEP:WHAT: GC removes an unreachable section under gc_sections=true and keeps it under
///   false; the reachable chain entry -> main -> add is always kept.
/// CEP:WHY: Required test (section GC by reachability).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn gc_removes_unreachable_section() {
    let mut dead = ObjectFile::new("dead.o");
    let t = dead
        .add_text_section(".text", 16, vec![0x8d, 0x04, 0x37, 0xc3])
        .unwrap();
    dead.add_symbol("add", true, true, Some(t), 0, 4).unwrap();
    let d = dead.add_data_section(".data.dead", 8, vec![0x55; 32]).unwrap();
    dead.add_symbol("dead_sym", true, true, Some(d), 0, 32).unwrap();

    let objs = [dead, main_object(), synthesize_start("main")];
    let with_gc = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
    let without_gc = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, false).unwrap();

    assert!(!with_gc.sections.iter().any(|s| s.name == ".data.dead"));
    assert_eq!(with_gc.sections.len(), 3); // add .text, main .text, synth .text
    assert!(without_gc.sections.iter().any(|s| s.name == ".data.dead"));
    assert_eq!(without_gc.sections.len(), 4);
    assert_eq!(with_gc.segments.len(), 1); // RX only; dead data was GC'd
    assert_eq!(without_gc.segments.len(), 2); // RX + RW

    // The kept image is still deterministic across runs.
    assert_eq!(
        link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap(),
        link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap()
    );
}

/// CEP:WHAT: Guards the pinned cross-agent shape of Reloc/RelocKind at the integration
///   layer.
/// CEP:WHY: rv8_backend constructs Reloc/RelocKind literally; a shape drift must break
///   loudly here.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: field order offset, kind, symbol, addend.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn pinned_reloc_shape() {
    let r = Reloc {
        offset: 11,
        kind: RelocKind::PcRel32,
        symbol: "add".to_string(),
        addend: -4,
    };
    assert_eq!(r.offset, 11);
    assert_eq!(r.kind, RelocKind::PcRel32);
    assert_eq!(r.symbol, "add");
    assert_eq!(r.addend, -4);
    assert_eq!(RelocKind::Abs64.elf_type(), 1);
    assert_eq!(RelocKind::PcRel32.elf_type(), 2);
}
