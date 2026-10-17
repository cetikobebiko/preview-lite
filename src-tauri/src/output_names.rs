//! User-facing names for multi-file outputs (split PDFs, page images).
//!
//! Job workers write their outputs under fixed private names inside the job
//! directory. The main process publishes each one under a name derived from the
//! document's file name, e.g. `rapor_sayfa_1-3.pdf` or `rapor_sayfa_2.png`.
//! Names are built to be valid on Windows, macOS and Linux filesystems.

/// Byte budget for the document part of a name. The longest suffix
/// (`_sayfa_4294967296-4294967296.pdf`) keeps the whole name far below the
/// common 255-byte component limit.
const OUTPUT_STEM_MAX_BYTES: usize = 120;
const OUTPUT_STEM_FALLBACK: &str = "belge";
const PDF_EXTENSION: &str = ".pdf";

/// One worker output and the name it is published under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlannedOutput {
    /// File name the job worker writes inside the job directory.
    pub(crate) worker_name: String,
    /// File name published into the user's output directory.
    pub(crate) target_name: String,
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Device names Windows reserves regardless of extension (`CON.txt` is `CON`).
fn is_windows_reserved(segment: &str) -> bool {
    let upper = segment.trim_end().to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let bytes = upper.as_bytes();
            bytes.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && (b'1'..=b'9').contains(&bytes[3])
        }
    }
}

fn trim_name_edges(value: &str) -> &str {
    value.trim_matches(|character: char| character == '.' || character.is_whitespace())
}

/// Document part of an output name, from the session's file name.
///
/// The last path component is taken and a trailing `.pdf` (any case) removed.
/// `< > : " / \ | ? *`, control and bidi direction characters become `_`, other
/// whitespace becomes a space, leading/trailing dots and spaces are trimmed,
/// the result is cut to [`OUTPUT_STEM_MAX_BYTES`] bytes, and a Windows device
/// name before the first dot gets a `_` appended. An empty result is `belge`.
pub(crate) fn output_name_stem(file_name: &str) -> String {
    let last_component = file_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let has_pdf_extension = last_component.len() >= PDF_EXTENSION.len()
        && last_component.is_char_boundary(last_component.len() - PDF_EXTENSION.len())
        && last_component[last_component.len() - PDF_EXTENSION.len()..]
            .eq_ignore_ascii_case(PDF_EXTENSION);
    let without_extension = if has_pdf_extension {
        &last_component[..last_component.len() - PDF_EXTENSION.len()]
    } else {
        last_component
    };
    let cleaned: String = without_extension
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() || is_bidi_control(character) => '_',
            character if character.is_whitespace() => ' ',
            character => character,
        })
        .collect();
    let stem = trim_name_edges(truncate_to_bytes(
        trim_name_edges(&cleaned),
        OUTPUT_STEM_MAX_BYTES,
    ));
    if stem.is_empty() {
        return OUTPUT_STEM_FALLBACK.to_string();
    }
    let (first_segment, rest) = stem.split_at(stem.find('.').unwrap_or(stem.len()));
    if is_windows_reserved(first_segment) {
        format!("{}_{rest}", first_segment.trim_end())
    } else {
        stem.to_string()
    }
}

/// `{stem}_sayfa_{n}.{extension}` for 0-based `page_index`.
pub(crate) fn page_image_output_name(stem: &str, page_index: u32, extension: &str) -> String {
    format!("{stem}_sayfa_{}.{extension}", u64::from(page_index) + 1)
}

/// `{stem}_sayfa_{start}-{end}.pdf`, or `{stem}_sayfa_{start}.pdf` for a
/// single page, from a 0-based inclusive range.
pub(crate) fn split_range_output_name(stem: &str, start: u32, end: u32) -> String {
    let first = u64::from(start) + 1;
    let last = u64::from(end) + 1;
    if first == last {
        format!("{stem}_sayfa_{first}.pdf")
    } else {
        format!("{stem}_sayfa_{first}-{last}.pdf")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_drops_directories_and_pdf_extension() {
        assert_eq!(output_name_stem("/srv/belgeler/rapor.pdf"), "rapor");
        assert_eq!(output_name_stem("C:\\Belgeler\\Rapor.PDF"), "Rapor");
        assert_eq!(output_name_stem("rapor.v2.pdf"), "rapor.v2");
        assert_eq!(output_name_stem("Sürüm 1.5"), "Sürüm 1.5");
        assert_eq!(output_name_stem("çizim.ai"), "çizim.ai");
    }

    #[test]
    fn stem_replaces_characters_invalid_on_any_platform() {
        assert_eq!(output_name_stem("a<b>c:d\"e|f?g*h.pdf"), "a_b_c_d_e_f_g_h");
        assert_eq!(output_name_stem("satır\nsonu\t.pdf"), "satır_sonu_");
        assert_eq!(output_name_stem("evil\u{202E}fdp.exe.pdf"), "evil_fdp.exe");
        assert_eq!(output_name_stem("geniş\u{00A0}boşluk.pdf"), "geniş boşluk");
    }

    #[test]
    fn stem_trims_hidden_file_dots_and_edge_whitespace() {
        assert_eq!(output_name_stem("..gizli.pdf"), "gizli");
        assert_eq!(output_name_stem("  rapor . .pdf"), "rapor");
        assert_eq!(output_name_stem("../../etc/passwd"), "passwd");
    }

    #[test]
    fn stem_falls_back_when_nothing_is_left() {
        for name in ["", ".pdf", "...", "   .pdf", "/", "dir/"] {
            assert_eq!(output_name_stem(name), "belge", "{name:?}");
        }
    }

    #[test]
    fn stem_neutralizes_windows_device_names() {
        assert_eq!(output_name_stem("CON.pdf"), "CON_");
        assert_eq!(output_name_stem("nul.pdf"), "nul_");
        assert_eq!(output_name_stem("com1.taslak.pdf"), "com1_.taslak");
        assert_eq!(output_name_stem("LPT9 .x.pdf"), "LPT9_.x");
        assert_eq!(output_name_stem("COM0.pdf"), "COM0");
        assert_eq!(output_name_stem("CONSOLE.pdf"), "CONSOLE");
    }

    #[test]
    fn stem_is_bounded_on_a_character_boundary() {
        let long = format!("{}.pdf", "ş".repeat(200));
        let stem = output_name_stem(&long);
        assert!(stem.len() <= OUTPUT_STEM_MAX_BYTES, "{}", stem.len());
        assert_eq!(stem, "ş".repeat(OUTPUT_STEM_MAX_BYTES / 2));

        let dotted = format!("{}. son.pdf", "a".repeat(OUTPUT_STEM_MAX_BYTES - 1));
        assert_eq!(
            output_name_stem(&dotted),
            "a".repeat(OUTPUT_STEM_MAX_BYTES - 1)
        );
    }

    #[test]
    fn page_and_range_names_are_one_based() {
        assert_eq!(
            page_image_output_name("rapor", 0, "png"),
            "rapor_sayfa_1.png"
        );
        assert_eq!(
            page_image_output_name("rapor", 9, "jpg"),
            "rapor_sayfa_10.jpg"
        );
        assert_eq!(
            split_range_output_name("rapor", 0, 2),
            "rapor_sayfa_1-3.pdf"
        );
        assert_eq!(split_range_output_name("rapor", 4, 4), "rapor_sayfa_5.pdf");
        assert_eq!(
            split_range_output_name("rapor", u32::MAX, u32::MAX),
            "rapor_sayfa_4294967296.pdf"
        );
        let longest = split_range_output_name(&"a".repeat(OUTPUT_STEM_MAX_BYTES), 0, u32::MAX - 1);
        assert!(longest.len() < 255, "{}", longest.len());
    }
}
