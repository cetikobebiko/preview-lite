//! CAP-03: revision boundaries from the raw bytes.
//!
//! PDFium has no revision API (§3.1 note) and lopdf drops `/Prev` on load (lopdf 0.45/0.46
//! finding), so revisions are found by a streaming byte scan: every `startxref <n> %%EOF`
//! whose `<n>` points at an `xref` table or an `N G obj` (xref stream) ends a revision. The
//! file is read in bounded chunks; it is never loaded whole.

use std::io::{Read, Seek, SeekFrom};

const CHUNK: usize = 64 * 1024;
/// `startxref`, whitespace and a 20-digit offset fit easily.
const LOOKBACK: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revisions {
    /// Offset after each revision's `%%EOF` and its end-of-line, ascending.
    pub ends: Vec<u64>,
    /// Offset just after each `%%EOF` (no end-of-line), same order as `ends`.
    pub marker_ends: Vec<u64>,
    /// `%%EOF` markers rejected because no valid `startxref` led to them.
    pub ignored_markers: u32,
    pub file_length: u64,
}

impl Revisions {
    /// The revision a ByteRange ending at `covered_end` covers. A signature may end before or
    /// after the end-of-line of `%%EOF`.
    pub fn index_ending_at(&self, covered_end: u64) -> Option<usize> {
        self.ends
            .iter()
            .zip(&self.marker_ends)
            .position(|(end, marker)| *end == covered_end || *marker == covered_end)
    }

    pub fn trailing_bytes(&self) -> u64 {
        self.file_length - self.ends.last().copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionScanError {
    TooManyRevisions,
    Io,
}

pub fn scan_revisions<R: Read + Seek>(
    reader: &mut R,
    file_length: u64,
    max_revisions: usize,
) -> Result<Revisions, RevisionScanError> {
    let candidates = find_markers(reader, max_revisions)?;
    let mut revisions = Revisions {
        ends: Vec::new(),
        marker_ends: Vec::new(),
        ignored_markers: 0,
        file_length,
    };
    for (marker_end, startxref) in candidates {
        if !points_at_xref_section(reader, startxref, marker_end)? {
            revisions.ignored_markers = revisions.ignored_markers.saturating_add(1);
            continue;
        }
        let end = marker_end + end_of_line_length(reader, marker_end)?;
        revisions.marker_ends.push(marker_end);
        revisions.ends.push(end);
    }
    Ok(revisions)
}

/// `(offset after %%EOF, startxref value)` of every syntactically valid trailer marker.
fn find_markers<R: Read + Seek>(
    reader: &mut R,
    max_revisions: usize,
) -> Result<Vec<(u64, u64)>, RevisionScanError> {
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| RevisionScanError::Io)?;
    let mut markers = Vec::new();
    let mut window: Vec<u8> = Vec::with_capacity(CHUNK + LOOKBACK);
    let mut window_start: u64 = 0;
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let read = reader.read(&mut chunk).map_err(|_| RevisionScanError::Io)?;
        if read == 0 {
            break;
        }
        let carried = window.len();
        window.extend_from_slice(&chunk[..read]);
        let mut search_from = 0;
        while let Some(found) = find(&window[search_from..], b"%%EOF") {
            let position = search_from + found;
            search_from = position + 1;
            // A marker wholly inside the carried bytes was handled with the previous chunk.
            if position + 5 <= carried {
                continue;
            }
            if let Some(startxref) = startxref_before(&window[..position]) {
                markers.push((window_start + position as u64 + 5, startxref));
                if markers.len() > max_revisions {
                    return Err(RevisionScanError::TooManyRevisions);
                }
            }
        }
        let keep = window.len().min(LOOKBACK);
        let drop = window.len() - keep;
        window.drain(..drop);
        window_start += drop as u64;
    }
    Ok(markers)
}

/// The integer of a `startxref <ws> <digits> <ws>` that ends right before `%%EOF`.
fn startxref_before(before: &[u8]) -> Option<u64> {
    let mut end = before.len();
    while end > 0 && is_whitespace(before[end - 1]) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && before[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start == end || end - start > 20 {
        return None;
    }
    let digits = std::str::from_utf8(&before[start..end]).ok()?;
    let offset = digits.parse::<u64>().ok()?;
    let mut keyword_end = start;
    let whitespace_end = keyword_end;
    while keyword_end > 0 && is_whitespace(before[keyword_end - 1]) {
        keyword_end -= 1;
    }
    if keyword_end == whitespace_end {
        return None;
    }
    before[..keyword_end]
        .ends_with(b"startxref")
        .then_some(offset)
}

fn points_at_xref_section<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    marker_end: u64,
) -> Result<bool, RevisionScanError> {
    if offset >= marker_end {
        return Ok(false);
    }
    let mut head = [0u8; 48];
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| RevisionScanError::Io)?;
    let read = read_up_to(reader, &mut head)?;
    let head = &head[..read];
    let skip = head.iter().take_while(|byte| is_whitespace(**byte)).count();
    let head = &head[skip..];
    if head.starts_with(b"xref") {
        return Ok(true);
    }
    // `N G obj`
    let mut parts = head
        .split(|byte| is_whitespace(*byte))
        .filter(|part| !part.is_empty());
    let number = parts
        .next()
        .is_some_and(|part| part.iter().all(u8::is_ascii_digit));
    let generation = parts
        .next()
        .is_some_and(|part| part.iter().all(u8::is_ascii_digit));
    let keyword = parts.next().is_some_and(|part| part.starts_with(b"obj"));
    Ok(number && generation && keyword)
}

fn end_of_line_length<R: Read + Seek>(
    reader: &mut R,
    marker_end: u64,
) -> Result<u64, RevisionScanError> {
    let mut next = [0u8; 2];
    reader
        .seek(SeekFrom::Start(marker_end))
        .map_err(|_| RevisionScanError::Io)?;
    let read = read_up_to(reader, &mut next)?;
    Ok(match &next[..read] {
        [b'\r', b'\n', ..] => 2,
        [b'\r' | b'\n', ..] => 1,
        _ => 0,
    })
}

fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<usize, RevisionScanError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return Err(RevisionScanError::Io),
        }
    }
    Ok(filled)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn revision(body: &str, offset_of_xref: usize) -> String {
        format!("{body}startxref\n{offset_of_xref}\n%%EOF\n")
    }

    fn scan(bytes: &[u8]) -> Revisions {
        scan_revisions(&mut Cursor::new(bytes), bytes.len() as u64, 10).unwrap()
    }

    #[test]
    fn finds_each_revision_end_with_its_end_of_line() {
        let first = revision("%PDF-1.7\nxref\n0 1\ntrailer\n<<>>\n", 9);
        let second_xref = first.len();
        let second = revision("xref\n0 1\ntrailer\n<<>>\n", second_xref);
        let file = format!("{first}{second}");
        let revisions = scan(file.as_bytes());
        assert_eq!(revisions.ends, vec![first.len() as u64, file.len() as u64]);
        assert_eq!(revisions.marker_ends[0], first.len() as u64 - 1);
        assert_eq!(revisions.index_ending_at(first.len() as u64 - 1), Some(0));
        assert_eq!(revisions.trailing_bytes(), 0);
    }

    #[test]
    fn counts_trailing_bytes_and_ignores_markers_without_a_valid_startxref() {
        let first = revision("%PDF-1.7\n1 0 obj\n<</Type/XRef>>\n", 9);
        let file = format!("{first}%%EOF\nstartxref\n999999\n%%EOF\ngarbage");
        let revisions = scan(file.as_bytes());
        assert_eq!(revisions.ends, vec![first.len() as u64]);
        assert_eq!(revisions.ignored_markers, 1);
        assert_eq!(
            revisions.trailing_bytes(),
            (file.len() - first.len()) as u64
        );
    }

    #[test]
    fn markers_split_across_chunks_are_found_once() {
        let padding = "%".repeat(CHUNK - 12);
        let body = format!("%PDF-1.7\nxref\n{padding}\n");
        let file = revision(&body, 9);
        let revisions = scan(file.as_bytes());
        assert_eq!(revisions.ends, vec![file.len() as u64]);
    }

    #[test]
    fn too_many_revisions_is_an_error() {
        let mut file = String::from("%PDF-1.7\nxref\n");
        for _ in 0..12 {
            file.push_str("startxref\n9\n%%EOF\n");
        }
        assert_eq!(
            scan_revisions(&mut Cursor::new(file.as_bytes()), file.len() as u64, 10),
            Err(RevisionScanError::TooManyRevisions)
        );
    }
}
