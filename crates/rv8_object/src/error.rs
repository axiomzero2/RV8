// CEP:FILE: crates/rv8_object/src/error.rs
// CEP:WHAT: The single failure type `ObjectError` returned by every fallible rv8_object API.
// CEP:WHY: Standard section 2.3 (fail-closed, never panic): model validation, string-table
//   growth, ELF emission, symbol resolution, layout, and relocation application all report
//   named, context-carrying errors so callers can match on variants instead of strings.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: none; this type *is* the failure channel for the crate.
// CEP:ASSUMES: Display text is diagnostic only; consumers must match on variants.
// CEP:COST: plain enum; context Strings allocate only on the error path, never on success.
// CEP:EVIDENCE: every variant is constructed by at least one unit or acceptance test inside
//   crates/rv8_object (see model::tests, elf::tests, link::tests, tests/acceptance.rs).

use std::fmt;

/// CEP:WHAT: Every failure rv8_object can report, with machine-matchable variants.
/// CEP:WHY: One error type keeps the public API uniform (`Result<_, ObjectError>`) and makes
///   truncated or malformed inputs impossible to turn into panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: variant set covers all named budgets in model.rs plus link/elf failure modes.
/// CEP:COST: size <= 64 bytes per value; construction allocates only context Strings.
/// CEP:EVIDENCE: unit tests rv8_object::model::tests, elf::tests, link::tests and integration
///   tests in crates/rv8_object/tests/acceptance.rs construct each rejection path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectError {
    /// Feature deliberately not implemented (dwarf/macho/coff serializers, hot/cold split).
    /// CEP:ASSUMES: callers treat this as "known stub", not "input was invalid".
    Unsupported(&'static str),
    /// A named budget constant (MAX_*) was exceeded.
    BudgetExceeded {
        /// Which named budget was hit (for example "MAX_SECTIONS").
        what: &'static str,
        /// The budget limit.
        limit: u64,
        /// The request that exceeded it.
        actual: u64,
    },
    /// Section alignment is neither zero (unspecified) nor a power of two.
    InvalidAlignment {
        /// Offending section name.
        section: String,
        /// Offending alignment.
        align: u64,
    },
    /// A single section exceeds `MAX_SECTION_BYTES`.
    SectionTooLarge {
        /// Offending section name.
        section: String,
        /// Requested size in bytes.
        size: u64,
    },
    /// A section/symbol/relocation name is invalid (too long or contains an interior NUL).
    InvalidName {
        /// Offending name.
        name: String,
        /// Why it was rejected ("name exceeds MAX_NAME_BYTES", "name contains interior NUL").
        reason: &'static str,
    },
    /// A symbol points at a section index that does not exist in its object.
    SymbolSectionOutOfRange {
        /// Symbol name.
        symbol: String,
        /// Section index carried by the symbol.
        section: usize,
        /// Number of sections that actually exist.
        sections: usize,
    },
    /// An undefined (referenced-only) symbol illegally carries a section index.
    UndefinedSymbolWithSection {
        /// Symbol name.
        name: String,
    },
    /// A defined symbol illegally lacks a section anchor.
    DefinedSymbolMissingSection {
        /// Symbol name.
        name: String,
    },
    /// A relocation would write past the end of its section contents.
    RelocOutOfBounds {
        /// Section name.
        section: String,
        /// Relocation offset.
        offset: u64,
        /// Number of bytes the relocation writes.
        write: u64,
        /// Size of the section contents.
        size: u64,
    },
    /// A relocation is attached to a SHT_NOBITS section, which has no file bytes to patch.
    RelocOnNoBits {
        /// Section name.
        section: String,
    },
    /// Two objects define the same symbol name as a strong global.
    DuplicateStrongSymbol {
        /// Duplicated symbol name.
        name: String,
    },
    /// A relocation references a symbol name that does not exist in its own object
    /// (reported by the relocatable writer; the linker reports UndefinedSymbol instead).
    UnknownRelocSymbol {
        /// Symbol name referenced by the relocation.
        name: String,
    },
    /// After section GC, a kept section still references a symbol with no definition.
    UndefinedSymbol {
        /// Symbol name that stayed undefined.
        name: String,
    },
    /// The requested entry symbol is not a defined global in any input object.
    MissingEntrySymbol {
        /// Entry symbol name that could not be resolved.
        name: String,
    },
    /// The requested image base is not page-aligned, which would break PT_LOAD
    /// p_vaddr = p_offset (mod p_align) congruence.
    InvalidBaseVaddr {
        /// The offending base address.
        base: u64,
    },
    /// A computed relocation value does not fit its field (PcRel32: i32; Abs64: u64).
    RelocOutOfRange {
        /// Symbol the relocation resolves against.
        symbol: String,
        /// Relocation kind name ("PcRel32" or "Abs64").
        kind: &'static str,
        /// The computed value that did not fit.
        value: i128,
    },
    /// Checked arithmetic overflowed during layout or emission.
    ArithmeticOverflow {
        /// Where the overflow happened ("align_up", "section cursor", ...).
        context: &'static str,
    },
    /// A bounded write hit the end of its output buffer (truncated image; defensive).
    WriteOutOfBounds {
        /// Write offset.
        offset: usize,
        /// Buffer length.
        len: usize,
        /// Bytes the write needed.
        need: usize,
    },
}

impl fmt::Display for ObjectError {
    /// CEP:WHAT: Human-readable rendering of every ObjectError variant.
    /// CEP:WHY: Diagnostics without losing variant-matchability.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: text is diagnostic only.
    /// CEP:COST: O(variant context size).
    /// CEP:EVIDENCE: unit test rv8_object::error::tests::display_is_shaped.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::Unsupported(what) => write!(f, "unsupported operation: {what}"),
            ObjectError::BudgetExceeded { what, limit, actual } => {
                write!(f, "budget {what} exceeded: limit {limit}, requested {actual}")
            }
            ObjectError::InvalidAlignment { section, align } => {
                write!(f, "section {section:?} alignment {align} is not a power of two")
            }
            ObjectError::SectionTooLarge { section, size } => {
                write!(f, "section {section:?} size {size} exceeds MAX_SECTION_BYTES")
            }
            ObjectError::InvalidName { name, reason } => {
                write!(f, "invalid name {name:?}: {reason}")
            }
            ObjectError::SymbolSectionOutOfRange { symbol, section, sections } => {
                write!(
                    f,
                    "symbol {symbol:?} references section {section} but object has {sections} section(s)"
                )
            }
            ObjectError::UndefinedSymbolWithSection { name } => {
                write!(f, "undefined symbol {name:?} illegally carries a section index")
            }
            ObjectError::DefinedSymbolMissingSection { name } => {
                write!(f, "defined symbol {name:?} lacks a section anchor")
            }
            ObjectError::RelocOutOfBounds { section, offset, write, size } => {
                write!(
                    f,
                    "relocation at offset {offset} writes {write} byte(s) past section {section:?} of size {size}"
                )
            }
            ObjectError::RelocOnNoBits { section } => {
                write!(f, "relocation attached to NOBITS section {section:?} has no bytes to patch")
            }
            ObjectError::DuplicateStrongSymbol { name } => {
                write!(f, "duplicate strong global definition of symbol {name:?}")
            }
            ObjectError::UnknownRelocSymbol { name } => {
                write!(f, "relocation references symbol {name:?} absent from its object")
            }
            ObjectError::UndefinedSymbol { name } => {
                write!(f, "undefined symbol {name:?} after section GC")
            }
            ObjectError::MissingEntrySymbol { name } => {
                write!(f, "entry symbol {name:?} is not a defined global in any input object")
            }
            ObjectError::InvalidBaseVaddr { base } => {
                write!(f, "image base {base:#x} is not page-aligned")
            }
            ObjectError::RelocOutOfRange { symbol, kind, value } => {
                write!(f, "relocation against {symbol:?} ({kind}) value {value} out of range")
            }
            ObjectError::ArithmeticOverflow { context } => {
                write!(f, "arithmetic overflow while computing {context}")
            }
            ObjectError::WriteOutOfBounds { offset, len, need } => {
                write!(f, "write at offset {offset} needs {need} byte(s), buffer holds {len}")
            }
        }
    }
}

impl std::error::Error for ObjectError {}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Pins the Display shape of representative variants.
    /// CEP:WHY: Diagnostics are part of the contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_is_shaped() {
        let e = ObjectError::DuplicateStrongSymbol { name: "main".into() };
        assert_eq!(e.to_string(), "duplicate strong global definition of symbol \"main\"");
        let e = ObjectError::BudgetExceeded { what: "MAX_SECTIONS", limit: 1024, actual: 1025 };
        assert_eq!(
            e.to_string(),
            "budget MAX_SECTIONS exceeded: limit 1024, requested 1025"
        );
        let e = ObjectError::RelocOutOfRange {
            symbol: "far".into(),
            kind: "PcRel32",
            value: 1 << 40,
        };
        assert!(e.to_string().contains("PcRel32"));
        assert_eq!(ObjectError::Unsupported("macho").to_string(), "unsupported operation: macho");
    }
}
