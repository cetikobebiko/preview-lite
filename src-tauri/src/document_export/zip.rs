// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.14.0, crates/export/src/zip.rs (MIT OR Apache-2.0, used under MIT).
// Changes from the original: deflate uses miniz_oxide (already a dependency) instead of flate2, and `add` lost its
// `compress` flag: an entry is deflated when that makes it smaller and stored otherwise.

//! A minimal zip writer (deflated or stored entries, no zip64) for Office files.

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Builds a zip archive in memory.
#[derive(Default)]
pub struct Zip {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Zip {
    /// Add a file, deflated when that makes it smaller (XML parts) and stored otherwise (PNG and
    /// JPEG images are already compressed).
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, body) = if deflated.len() < data.len() {
            (8u16, deflated.as_slice())
        } else {
            (0u16, data)
        };
        let offset = self.out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(sig.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // version needed
            h.extend(0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend(method.to_le_bytes());
            h.extend([0, 0, 0x21, 0]); // 00:00, 1980-01-01 (a zero date is not a valid DOS date)
            h.extend(crc.to_le_bytes());
            h.extend((body.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        self.out.extend(header(0x0403_4b50, false));
        self.out.extend(body);
        self.central.extend(header(0x0201_4b50, true));
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let start = self.out.len() as u32;
        let size = self.central.len() as u32;
        self.out.append(&mut self.central);
        self.out.extend(0x0605_4b50u32.to_le_bytes());
        self.out.extend([0u8; 4]);
        self.out.extend(self.count.to_le_bytes());
        self.out.extend(self.count.to_le_bytes());
        self.out.extend(size.to_le_bytes());
        self.out.extend(start.to_le_bytes());
        self.out.extend(0u16.to_le_bytes());
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn archive_has_both_directories() {
        let mut z = Zip::default();
        z.add("a.txt", b"hello hello hello");
        z.add("b.png", b"\x89PNG");
        let b = z.finish();
        assert!(b.starts_with(b"PK\x03\x04"));
        assert_eq!(b.windows(4).filter(|w| *w == b"PK\x01\x02").count(), 2);
        assert_eq!(&b[b.len() - 22..b.len() - 18], b"PK\x05\x06");
    }

    #[test]
    fn compressible_entries_are_deflated_and_inflate_back() {
        let text = "<w:p>merhaba dünya</w:p>".repeat(200);
        let mut z = Zip::default();
        z.add("word/document.xml", text.as_bytes());
        z.add("media/image1.png", b"\x89PNG");
        let b = z.finish();
        // Local header: method at 8, compressed size at 18, name length at 26.
        let method = u16::from_le_bytes([b[8], b[9]]);
        let size = u32::from_le_bytes([b[18], b[19], b[20], b[21]]) as usize;
        let name_len = u16::from_le_bytes([b[26], b[27]]) as usize;
        assert_eq!(method, 8);
        assert!(size < text.len() / 10, "{size}");
        let body = &b[30 + name_len..30 + name_len + size];
        assert_eq!(miniz_oxide::inflate::decompress_to_vec(body).unwrap(), text.as_bytes());
        // The tiny PNG stays stored.
        let second = 30 + name_len + size;
        assert_eq!(u16::from_le_bytes([b[second + 8], b[second + 9]]), 0);
    }
}
