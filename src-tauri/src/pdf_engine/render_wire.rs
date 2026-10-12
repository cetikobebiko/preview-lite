//! Binary IPC wire format (`render-frame/1`) for `render_session_page` and
//! `render_session_thumbnail` success responses.
//!
//! The frame is returned as a raw `tauri::ipc::Response` body so the RGBA
//! pixels reach the webview as an `ArrayBuffer` instead of a JSON number
//! array. Failures keep the JSON `DocumentSessionError` contract. The decoder
//! lives in `src/services/backend/renderFrame.js`.
//!
//! All integers are little-endian:
//!
//! | offset | size        | field                                   |
//! |-------:|------------:|-----------------------------------------|
//! | 0      | 4           | magic `AGRF`                            |
//! | 4      | 2           | version (`u16`) = 1                     |
//! | 6      | 4           | width (`u32`)                           |
//! | 10     | 4           | height (`u32`)                          |
//! | 14     | 4           | page index (`u32`)                      |
//! | 18     | 8           | generation (`u64`, <= `MAX_SAFE_INTEGER`) |
//! | 26     | 2           | request ID byte length (`u16`, 1..=512) |
//! | 28     | n           | request ID (UTF-8)                      |
//! | 28 + n | w * h * 4   | RGBA pixels                             |

use super::render_service::{RenderResponse, MAX_IDENTIFIER_LENGTH, MAX_SAFE_INTEGER};

pub const RENDER_FRAME_MAGIC: [u8; 4] = *b"AGRF";
pub const RENDER_FRAME_VERSION: u16 = 1;
pub const RENDER_FRAME_FIXED_HEADER_BYTES: usize = 28;
/// Request IDs are limited to `MAX_IDENTIFIER_LENGTH` characters; a UTF-8
/// character is at most four bytes.
pub const MAX_REQUEST_ID_BYTES: usize = MAX_IDENTIFIER_LENGTH * 4;

pub fn encode_render_frame(response: &RenderResponse) -> Result<Vec<u8>, String> {
    let request_id = response.request_id.as_bytes();
    if response.width == 0
        || response.height == 0
        || response.generation > MAX_SAFE_INTEGER
        || request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID_BYTES
    {
        return Err("render_invalid_response_metadata".to_string());
    }
    let expected_pixels = (response.width as usize)
        .checked_mul(response.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if expected_pixels != Some(response.data.len()) {
        return Err("render_invalid_rgba_payload".to_string());
    }

    let header_len = RENDER_FRAME_FIXED_HEADER_BYTES + request_id.len();
    let mut frame = Vec::with_capacity(header_len + response.data.len());
    frame.extend_from_slice(&RENDER_FRAME_MAGIC);
    frame.extend_from_slice(&RENDER_FRAME_VERSION.to_le_bytes());
    frame.extend_from_slice(&response.width.to_le_bytes());
    frame.extend_from_slice(&response.height.to_le_bytes());
    frame.extend_from_slice(&response.page_index.to_le_bytes());
    frame.extend_from_slice(&response.generation.to_le_bytes());
    frame.extend_from_slice(&(request_id.len() as u16).to_le_bytes());
    frame.extend_from_slice(request_id);
    debug_assert_eq!(frame.len(), header_len);
    frame.extend_from_slice(&response.data);
    Ok(frame)
}

/// Rust mirror of `decodeRenderFrame` (JS). Production code only encodes;
/// this exists so tests can verify command output byte-for-byte.
pub fn decode_render_frame(frame: &[u8]) -> Result<RenderResponse, String> {
    let invalid = || "render_invalid_rgba_payload".to_string();
    let header = frame
        .get(..RENDER_FRAME_FIXED_HEADER_BYTES)
        .ok_or_else(invalid)?;
    let u16_at = |offset: usize| u16::from_le_bytes([header[offset], header[offset + 1]]);
    let u32_at = |offset: usize| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    if header[..4] != RENDER_FRAME_MAGIC || u16_at(4) != RENDER_FRAME_VERSION {
        return Err(invalid());
    }
    let width = u32_at(6);
    let height = u32_at(10);
    let page_index = u32_at(14);
    let generation = u64::from_le_bytes(header[18..26].try_into().unwrap());
    let request_id_len = u16_at(26) as usize;
    if width == 0
        || height == 0
        || generation > MAX_SAFE_INTEGER
        || request_id_len == 0
        || request_id_len > MAX_REQUEST_ID_BYTES
    {
        return Err(invalid());
    }
    let pixel_offset = RENDER_FRAME_FIXED_HEADER_BYTES + request_id_len;
    let request_id = frame
        .get(RENDER_FRAME_FIXED_HEADER_BYTES..pixel_offset)
        .ok_or_else(invalid)?;
    let request_id = String::from_utf8(request_id.to_vec()).map_err(|_| invalid())?;
    let pixels = &frame[pixel_offset..];
    let expected_pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if expected_pixels != Some(pixels.len()) {
        return Err(invalid());
    }
    Ok(RenderResponse {
        width,
        height,
        data: std::sync::Arc::new(pixels.to_vec()),
        page_index,
        generation,
        request_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> RenderResponse {
        RenderResponse {
            width: 2,
            height: 1,
            data: std::sync::Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            page_index: 3,
            generation: 9,
            request_id: "main_page-request-1".to_string(),
        }
    }

    fn assert_same(left: &RenderResponse, right: &RenderResponse) {
        assert_eq!(left.width, right.width);
        assert_eq!(left.height, right.height);
        assert_eq!(left.data, right.data);
        assert_eq!(left.page_index, right.page_index);
        assert_eq!(left.generation, right.generation);
        assert_eq!(left.request_id, right.request_id);
    }

    #[test]
    fn encodes_golden_frame() {
        // Same bytes are decoded by `src/services/backend/renderFrame.test.js`.
        let frame = encode_render_frame(&RenderResponse {
            width: 1,
            height: 1,
            data: std::sync::Arc::new(vec![10, 20, 30, 255]),
            page_index: 2,
            generation: 7,
            request_id: "r1".to_string(),
        })
        .unwrap();
        assert_eq!(
            frame,
            [
                0x41, 0x47, 0x52, 0x46, // magic
                0x01, 0x00, // version
                0x01, 0x00, 0x00, 0x00, // width
                0x01, 0x00, 0x00, 0x00, // height
                0x02, 0x00, 0x00, 0x00, // page index
                0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // generation
                0x02, 0x00, // request ID length
                0x72, 0x31, // "r1"
                0x0a, 0x14, 0x1e, 0xff, // RGBA
            ]
        );
    }

    #[test]
    fn round_trips_frame() {
        let original = response();
        let frame = encode_render_frame(&original).unwrap();
        assert_eq!(
            frame.len(),
            RENDER_FRAME_FIXED_HEADER_BYTES + original.request_id.len() + original.data.len()
        );
        assert_same(&decode_render_frame(&frame).unwrap(), &original);
    }

    #[test]
    fn round_trips_boundary_metadata() {
        let original = RenderResponse {
            page_index: u32::MAX,
            generation: MAX_SAFE_INTEGER,
            request_id: format!(
                "{}{}",
                "ğ".repeat(MAX_IDENTIFIER_LENGTH / 2),
                "🙂".repeat(MAX_IDENTIFIER_LENGTH / 2)
            ),
            ..response()
        };
        assert_eq!(original.request_id.chars().count(), MAX_IDENTIFIER_LENGTH);
        let frame = encode_render_frame(&original).unwrap();
        assert_same(&decode_render_frame(&frame).unwrap(), &original);
    }

    #[test]
    fn rejects_inconsistent_pixel_length() {
        let mut short = response();
        std::sync::Arc::make_mut(&mut short.data).pop();
        assert_eq!(
            encode_render_frame(&short).unwrap_err(),
            "render_invalid_rgba_payload"
        );
    }

    #[test]
    fn rejects_invalid_metadata() {
        let cases = [
            RenderResponse {
                width: 0,
                data: Default::default(),
                ..response()
            },
            RenderResponse {
                generation: MAX_SAFE_INTEGER + 1,
                ..response()
            },
            RenderResponse {
                request_id: String::new(),
                ..response()
            },
            RenderResponse {
                request_id: "r".repeat(MAX_REQUEST_ID_BYTES + 1),
                ..response()
            },
        ];
        for case in cases {
            assert_eq!(
                encode_render_frame(&case).unwrap_err(),
                "render_invalid_response_metadata"
            );
        }
    }

    #[test]
    fn decoder_rejects_corrupted_frames() {
        let frame = encode_render_frame(&response()).unwrap();
        assert!(decode_render_frame(&frame[..RENDER_FRAME_FIXED_HEADER_BYTES - 1]).is_err());
        assert!(decode_render_frame(&frame[..frame.len() - 1]).is_err());
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(decode_render_frame(&trailing).is_err());
        let mut wrong_magic = frame.clone();
        wrong_magic[0] = 0;
        assert!(decode_render_frame(&wrong_magic).is_err());
        let mut wrong_version = frame.clone();
        wrong_version[4] = 2;
        assert!(decode_render_frame(&wrong_version).is_err());
        let mut wrong_width = frame.clone();
        wrong_width[6] = 3;
        assert!(decode_render_frame(&wrong_width).is_err());
        let mut unsafe_generation = frame.clone();
        unsafe_generation[18..26].copy_from_slice(&(MAX_SAFE_INTEGER + 1).to_le_bytes());
        assert!(decode_render_frame(&unsafe_generation).is_err());
    }
}
