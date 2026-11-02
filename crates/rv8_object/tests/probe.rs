// CEP:FILE: crates/rv8_object/tests/probe.rs
// CEP:WHAT: Developer probe: hexdumps one minimal Mach-O and one minimal COFF emission to
//   stdout for eyeballing byte layout during serializer work.
// CEP:WHY: Byte-level debugging aid next to the writers; not an acceptance check.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; unwrap on writer error (panics are permitted in tests).
// CEP:ASSUMES: none
// CEP:COST: test-only; fixtures are tiny.
// CEP:EVIDENCE: this test (assertion-free dump; assertions live in serializers.rs and
//   macho_serialization.rs).

use rv8_object::macho::write_macho64;
use rv8_object::coff::write_coff;
use rv8_object::model::ObjectFile;

fn macho_minimal() -> ObjectFile {
    let mut o = ObjectFile::new("m.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).unwrap();
    o.add_symbol("f", true, false, Some(t), 0, 1).unwrap();
    o
}

fn coff_minimal() -> ObjectFile {
    let mut o = ObjectFile::new("c.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 1).unwrap();
    o
}

#[test]
fn probe_dump() {
    let m = write_macho64(&macho_minimal()).unwrap();
    println!("macho len {}", m.len());
    for (i, chunk) in m.chunks(16).enumerate() {
        print!("{:04x}:", i * 16);
        for b in chunk { print!(" {:02x}", b); }
        for b in chunk { if b.is_ascii_graphic() || *b == b' ' { print!("{}", *b as char); } else { print!("."); } }
        println!();
    }
    let c = write_coff(&coff_minimal()).unwrap();
    println!("coff len {}", c.len());
    for (i, chunk) in c.chunks(16).enumerate() {
        print!("{:04x}:", i * 16);
        for b in chunk { print!(" {:02x}", b); }
        for b in chunk { if b.is_ascii_graphic() || *b == b' ' { print!("{}", *b as char); } else { print!("."); } }
        println!();
    }
}
