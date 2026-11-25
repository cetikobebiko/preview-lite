//! Resource limits of the signature verifier (Gate 1 §E). Every input is hostile.
//!
//! Document-level limits fail the job with [`SIGNATURE_RESOURCE_LIMIT_EXCEEDED`];
//! per-signature limits make that signature `Malformed`/`Indeterminate` with a finding.
//! String limits are not errors: values are trimmed.

pub const SIGNATURE_RESOURCE_LIMIT_EXCEEDED: &str = "SIGNATURE_RESOURCE_LIMIT_EXCEEDED";
pub const SIGNATURE_INPUT_UNSUPPORTED: &str = "SIGNATURE_INPUT_UNSUPPORTED";

/// Document: `startxref … %%EOF` markers.
pub const MAX_REVISIONS: usize = 1_000;
/// Document: signature objects (PDFium) or signature dictionaries (lopdf).
pub const MAX_SIGNATURES: usize = 256;
/// Signature: decoded `/Contents` bytes.
pub const MAX_CONTENTS_BYTES: usize = 1024 * 1024;
/// Signature: certificates carried in the CMS.
pub const MAX_CMS_CERTIFICATES: usize = 32;
/// Signature: ASN.1 nesting depth of the CMS.
pub const MAX_DER_DEPTH: usize = 64;
/// Signature: ASN.1 elements of the CMS.
pub const MAX_DER_ELEMENTS: usize = 100_000;
/// Document: bytes hashed for all signatures together.
pub const MAX_TOTAL_HASHED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Document: lopdf structural walk input (same as the other structural inspections).
pub const MAX_STRUCTURAL_INPUT_BYTES: u64 =
    crate::document_structure::MAX_STRUCTURE_INPUT_BYTES as u64;
/// RSA modulus range the verifier accepts at all (policy is in `policy.rs`).
pub const MAX_RSA_MODULUS_BITS: usize = 8192;

pub const MAX_FIELD_NAME_CHARS: usize = 256;
pub const MAX_REASON_CHARS: usize = 512;
pub const MAX_NAME_COMPONENT_CHARS: usize = 256;
pub const MAX_CONTEXT_CHARS: usize = 512;
pub const MAX_FINDINGS_PER_SIGNATURE: usize = 64;
pub const MAX_GLOBAL_FINDINGS: usize = 64;
/// Upper bound of the serialized report checked by the main process.
pub const MAX_REPORT_JSON_BYTES: usize = 4 * 1024 * 1024;

/// Trims to `max` characters (Unicode scalar values), marking the cut with `…`. Control
/// characters are replaced so a hostile string cannot shape the UI or a log line.
pub fn trim(value: &str, max: usize) -> String {
    let mut out = String::new();
    for (count, character) in value.chars().enumerate() {
        if count >= max {
            out.push('…');
            return out;
        }
        out.push(if character.is_control() {
            '\u{FFFD}'
        } else {
            character
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_cuts_marks_and_neutralises_control_characters() {
        assert_eq!(trim("abc", 3), "abc");
        assert_eq!(trim("abcd", 3), "abc…");
        assert_eq!(trim("a\nb\u{1b}", 10), "a\u{FFFD}b\u{FFFD}");
        assert_eq!(trim("çğüşöı", 4), "çğüş…");
    }
}
