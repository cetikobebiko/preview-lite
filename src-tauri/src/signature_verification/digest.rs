//! CAP-04: digest of the ByteRange bytes, streamed through a bounded buffer.
//!
//! Only SHA-256/384/512 are computed. SHA-1 and MD5 are disallowed by policy (§8) and their
//! crates are not approved (§20), so a signature using them is reported, not hashed.

use sha2::Digest;
use std::io::{Read, Seek, SeekFrom};

const BUFFER: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlgorithm {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Sha384 => "SHA-384",
            Self::Sha512 => "SHA-512",
        }
    }

    pub fn hash(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => sha2::Sha256::digest(bytes).to_vec(),
            Self::Sha384 => sha2::Sha384::digest(bytes).to_vec(),
            Self::Sha512 => sha2::Sha512::digest(bytes).to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestError {
    /// The document-wide hashing budget is used up.
    BudgetExceeded,
    Io,
}

/// Hashes `ranges` of `reader` in order. `budget` is the number of bytes this document may
/// still hash; it is reduced by what this call hashes.
pub fn digest_ranges<R: Read + Seek>(
    reader: &mut R,
    ranges: &[(u64, u64)],
    algorithm: DigestAlgorithm,
    budget: &mut u64,
) -> Result<Vec<u8>, DigestError> {
    let total = ranges
        .iter()
        .try_fold(0u64, |sum, (_, length)| sum.checked_add(*length))
        .ok_or(DigestError::BudgetExceeded)?;
    if total > *budget {
        return Err(DigestError::BudgetExceeded);
    }
    *budget -= total;
    match algorithm {
        DigestAlgorithm::Sha256 => stream::<sha2::Sha256, R>(reader, ranges),
        DigestAlgorithm::Sha384 => stream::<sha2::Sha384, R>(reader, ranges),
        DigestAlgorithm::Sha512 => stream::<sha2::Sha512, R>(reader, ranges),
    }
}

fn stream<D: Digest, R: Read + Seek>(
    reader: &mut R,
    ranges: &[(u64, u64)],
) -> Result<Vec<u8>, DigestError> {
    let mut hasher = D::new();
    let mut buffer = vec![0u8; BUFFER];
    for &(offset, length) in ranges {
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(|_| DigestError::Io)?;
        let mut remaining = length;
        while remaining > 0 {
            let want = usize::try_from(remaining.min(BUFFER as u64)).unwrap_or(BUFFER);
            reader
                .read_exact(&mut buffer[..want])
                .map_err(|_| DigestError::Io)?;
            hasher.update(&buffer[..want]);
            remaining -= want as u64;
        }
    }
    Ok(hasher.finalize().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn streamed_digest_equals_the_digest_of_the_concatenated_ranges() {
        let file: Vec<u8> = (0..200_000u32).map(|value| (value % 251) as u8).collect();
        let ranges = [(0u64, 70_000u64), (100_000, 100_000)];
        let mut expected = file[..70_000].to_vec();
        expected.extend_from_slice(&file[100_000..]);
        for algorithm in [
            DigestAlgorithm::Sha256,
            DigestAlgorithm::Sha384,
            DigestAlgorithm::Sha512,
        ] {
            let mut budget = u64::MAX;
            let digest =
                digest_ranges(&mut Cursor::new(&file), &ranges, algorithm, &mut budget).unwrap();
            assert_eq!(digest, algorithm.hash(&expected), "{}", algorithm.name());
            assert_eq!(budget, u64::MAX - 170_000);
        }
    }

    #[test]
    fn budget_and_short_files_are_errors() {
        let file = vec![0u8; 100];
        let mut budget = 50;
        assert_eq!(
            digest_ranges(
                &mut Cursor::new(&file),
                &[(0, 60)],
                DigestAlgorithm::Sha256,
                &mut budget
            ),
            Err(DigestError::BudgetExceeded)
        );
        assert_eq!(budget, 50);
        let mut budget = u64::MAX;
        assert_eq!(
            digest_ranges(
                &mut Cursor::new(&file),
                &[(90, 20)],
                DigestAlgorithm::Sha256,
                &mut budget
            ),
            Err(DigestError::Io)
        );
    }
}
