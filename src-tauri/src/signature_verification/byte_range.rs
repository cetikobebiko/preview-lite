//! CAP-02: ByteRange arithmetic and the `/Contents` gap (design §7).
//!
//! `[a b c d]` must satisfy: four non-negative integers, no overflow, `a = 0`,
//! `a + b < c`, `c + d ≤ file length`; the gap `[b, c)` is exactly the `/Contents`
//! hex string (`<` at `b`, `>` at `c - 1`, hex digits only in between) and decodes to the
//! bytes PDFium reports. Whether `c + d` ends a revision is checked by the caller with
//! [`super::revisions`].

use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedByteRange {
    pub first: (u64, u64),
    pub second: (u64, u64),
}

impl CheckedByteRange {
    /// Offset just after the last covered byte.
    pub fn covered_end(&self) -> u64 {
        // Checked in `check_arithmetic`.
        self.second.0 + self.second.1
    }

    pub fn ranges(&self) -> [(u64, u64); 2] {
        [self.first, self.second]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRangeViolation {
    /// Not four integers; more than one exclusion gap is rule 53.
    WrongLength,
    Negative,
    Overflow,
    FirstOffsetNotZero,
    /// Ranges overlap, are out of order, or leave no room for `<>`.
    OverlapOrUnordered,
    OutOfBounds,
    /// `b` does not point at `<` or `c - 1` at `>`.
    GapNotContents,
    /// The gap holds something other than hex digits, or an odd number of them.
    GapNotHex,
    /// The gap is larger than the `/Contents` limit.
    ContentsTooLarge,
    /// The decoded gap differs from the `/Contents` value PDFium reports.
    ContentsMismatch,
    /// `c + d` is not the end of a revision: some bytes of that revision are not covered.
    NotAtRevisionEnd,
}

impl ByteRangeViolation {
    /// §14 code of the violation.
    pub fn code(self) -> &'static str {
        match self {
            Self::NotAtRevisionEnd | Self::GapNotContents | Self::GapNotHex => {
                "SIGNATURE_BYTE_RANGE_GAP_DETECTED"
            }
            Self::ContentsTooLarge => super::limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
            _ => "SIGNATURE_BYTE_RANGE_INVALID",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::WrongLength => "ByteRange dört tamsayıdan oluşmuyor",
            Self::Negative => "ByteRange negatif değer içeriyor",
            Self::Overflow => "ByteRange toplamı taşıyor",
            Self::FirstOffsetNotZero => "ByteRange dosyanın başından başlamıyor",
            Self::OverlapOrUnordered => "ByteRange aralıkları çakışıyor ya da sırasız",
            Self::OutOfBounds => "ByteRange dosya sonunu aşıyor",
            Self::GapNotContents => "ByteRange boşluğu /Contents değerine denk gelmiyor",
            Self::GapNotHex => "ByteRange boşluğunda onaltılık olmayan bayt var",
            Self::ContentsTooLarge => "/Contents boyut sınırını aşıyor",
            Self::ContentsMismatch => {
                "ByteRange boşluğu PDFium'un okuduğu /Contents ile aynı değil"
            }
            Self::NotAtRevisionEnd => "ByteRange imzanın revizyonunun sonunda bitmiyor",
        }
    }
}

pub fn check_arithmetic(
    values: &[i64],
    file_length: u64,
) -> Result<CheckedByteRange, ByteRangeViolation> {
    let [a, b, c, d] = <[i64; 4]>::try_from(values).map_err(|_| ByteRangeViolation::WrongLength)?;
    if [a, b, c, d].iter().any(|value| *value < 0) {
        return Err(ByteRangeViolation::Negative);
    }
    let [a, b, c, d] = [a, b, c, d].map(|value| value.unsigned_abs());
    if a != 0 {
        return Err(ByteRangeViolation::FirstOffsetNotZero);
    }
    let first_end = a.checked_add(b).ok_or(ByteRangeViolation::Overflow)?;
    let second_end = c.checked_add(d).ok_or(ByteRangeViolation::Overflow)?;
    // The gap holds at least `<>`.
    if first_end.checked_add(2).is_none_or(|minimum| minimum > c) {
        return Err(ByteRangeViolation::OverlapOrUnordered);
    }
    if second_end > file_length {
        return Err(ByteRangeViolation::OutOfBounds);
    }
    Ok(CheckedByteRange {
        first: (a, b),
        second: (c, d),
    })
}

/// Reads the gap `[b, c)` and returns the decoded `/Contents` bytes.
pub fn read_contents_gap<R: Read + Seek>(
    reader: &mut R,
    range: &CheckedByteRange,
    max_decoded: usize,
) -> Result<Vec<u8>, ByteRangeViolation> {
    let start = range.first.1;
    let end = range.second.0;
    let gap = end - start; // `check_arithmetic` guarantees end ≥ start + 2.
    let max_gap = u64::try_from(max_decoded)
        .ok()
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|digits| digits.checked_add(2))
        .ok_or(ByteRangeViolation::ContentsTooLarge)?;
    if gap > max_gap {
        return Err(ByteRangeViolation::ContentsTooLarge);
    }
    let gap = usize::try_from(gap).map_err(|_| ByteRangeViolation::ContentsTooLarge)?;
    let mut bytes = vec![0u8; gap];
    reader
        .seek(SeekFrom::Start(start))
        .and_then(|_| reader.read_exact(&mut bytes))
        .map_err(|_| ByteRangeViolation::OutOfBounds)?;
    decode_hex_gap(&bytes)
}

/// `<hex…>` → bytes. Whitespace is not accepted: a signature writer reserves the exact gap.
pub fn decode_hex_gap(gap: &[u8]) -> Result<Vec<u8>, ByteRangeViolation> {
    let inner = gap
        .strip_prefix(b"<")
        .and_then(|rest| rest.strip_suffix(b">"))
        .ok_or(ByteRangeViolation::GapNotContents)?;
    if inner.len() % 2 != 0 {
        return Err(ByteRangeViolation::GapNotHex);
    }
    inner
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[high, low]| match (hex_value(high), hex_value(low)) {
            (Some(high), Some(low)) => Ok(high << 4 | low),
            _ => Err(ByteRangeViolation::GapNotHex),
        })
        .collect()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn arithmetic_accepts_a_well_formed_range() {
        let range = check_arithmetic(&[0, 10, 20, 5], 25).unwrap();
        assert_eq!(range.covered_end(), 25);
        assert_eq!(range.ranges(), [(0, 10), (20, 5)]);
    }

    #[test]
    fn arithmetic_rejects_every_rule() {
        use ByteRangeViolation::*;
        let cases: [(&[i64], u64, ByteRangeViolation); 9] = [
            (&[0, 10, 20], 30, WrongLength),
            (&[0, 10, 20, 5, 30, 1], 40, WrongLength),
            (&[0, -1, 20, 5], 30, Negative),
            (&[5, 10, 20, 5], 30, FirstOffsetNotZero),
            (&[0, 20, 10, 5], 30, OverlapOrUnordered),
            (&[0, 10, 11, 5], 30, OverlapOrUnordered),
            (&[0, 10, 20, 11], 30, OutOfBounds),
            // Two non-negative i64 cannot overflow u64; the sum is out of bounds instead.
            (&[0, 10, i64::MAX, i64::MAX], 30, OutOfBounds),
            (&[0, i64::MAX, 20, 5], 30, OverlapOrUnordered),
        ];
        for (values, length, expected) in cases {
            assert_eq!(
                check_arithmetic(values, length),
                Err(expected),
                "{values:?}"
            );
        }
    }

    #[test]
    fn gap_must_be_exactly_a_hex_string() {
        assert_eq!(decode_hex_gap(b"<3082>").unwrap(), vec![0x30, 0x82]);
        assert_eq!(decode_hex_gap(b"<>").unwrap(), Vec::<u8>::new());
        assert_eq!(
            decode_hex_gap(b"(3082)"),
            Err(ByteRangeViolation::GapNotContents)
        );
        assert_eq!(decode_hex_gap(b"<308>"), Err(ByteRangeViolation::GapNotHex));
        assert_eq!(
            decode_hex_gap(b"<30 82>"),
            Err(ByteRangeViolation::GapNotHex)
        );
        assert_eq!(
            decode_hex_gap(b"<30zz>"),
            Err(ByteRangeViolation::GapNotHex)
        );
    }

    #[test]
    fn gap_is_read_from_the_file_and_bounded() {
        let file = b"0123456789<abcd>tail".to_vec();
        let range = check_arithmetic(&[0, 10, 16, 4], 20).unwrap();
        let decoded = read_contents_gap(&mut Cursor::new(&file), &range, 16).unwrap();
        assert_eq!(decoded, vec![0xab, 0xcd]);
        assert_eq!(
            read_contents_gap(&mut Cursor::new(&file), &range, 1),
            Err(ByteRangeViolation::ContentsTooLarge)
        );
        let shifted = check_arithmetic(&[0, 9, 16, 4], 20).unwrap();
        assert_eq!(
            read_contents_gap(&mut Cursor::new(&file), &shifted, 16),
            Err(ByteRangeViolation::GapNotContents)
        );
    }
}
