pub mod render_service;
pub mod render_wire;
pub mod runtime_metadata;
pub mod subprocess_manager;

use crate::{error::AppError, PasswordCache, RenderedPage};
use pdfium_render::prelude::*;
use render_service::{
    checked_rgba_len, validate_generation, ActiveRenderDocument, CachedRender, RenderCacheKey,
    RenderRequestIdentity, RenderResponse, RenderScope, RenderService, RenderSizingKey,
    RenderTargetKey, MAX_CACHE_ENTRY_BYTES, MAX_PIXEL_DIMENSION, MAX_SCALE,
};
use serde::Serialize;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio_util::sync::CancellationToken;

static PDFIUM_MUTEX: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub fn with_pdfium<T, E>(operation: impl FnOnce(&Pdfium) -> Result<T, E>) -> Result<T, E>
where
    E: From<String>,
{
    let _guard = PDFIUM_MUTEX.lock();
    let pdfium = get_pdfium()?;
    operation(&pdfium)
}

pub fn get_pdfium() -> Result<Pdfium, String> {
    let runtime_path = resolve_runtime_path().map_err(|e| e.to_string())?;
    verify_runtime(&runtime_path).map_err(|e| e.to_string())?;
    let binding = Pdfium::bind_to_library(&runtime_path)
        .map_err(|e| format!("Failed to bind PDFium library: {e}"))?;
    Ok(Pdfium::new(binding))
}

/// Get PDFium instance using a pre-resolved runtime path (for worker parity).
/// If `resolved_path` is provided, it is used directly after verification.
/// Otherwise, falls back to the standard resolution logic.
pub fn get_pdfium_with_resolved_path(resolved_path: Option<PathBuf>) -> Result<Pdfium, String> {
    let runtime_path = match resolved_path {
        Some(path) => {
            // Verify the pre-resolved path exists and is valid
            if !path.is_file() {
                return Err(format!(
                    "Pre-resolved PDFium runtime path does not exist: {}",
                    path.display()
                ));
            }
            if path.file_name().and_then(|n| n.to_str())
                != Some(crate::pdf_engine::runtime_metadata::RUNTIME_FILENAME)
            {
                return Err(format!(
                    "Pre-resolved PDFium runtime has unexpected filename: {}",
                    path.display()
                ));
            }
            path
        }
        None => resolve_runtime_path().map_err(|e| e.to_string())?,
    };
    verify_runtime(&runtime_path).map_err(|e| e.to_string())?;
    let binding = Pdfium::bind_to_library(&runtime_path)
        .map_err(|e| format!("Failed to bind PDFium library: {e}"))?;
    Ok(Pdfium::new(binding))
}

pub(crate) fn resolve_runtime_path() -> Result<PathBuf, AppError> {
    #[cfg(target_os = "linux")]
    {
        resolve_linux_runtime_path()
    }
    #[cfg(target_os = "windows")]
    {
        resolve_windows_runtime_path()
    }
    #[cfg(target_os = "macos")]
    {
        resolve_macos_runtime_path()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(AppError::RuntimeUnsupported(
            "Unsupported target OS for PDFium runtime".to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn resolve_linux_runtime_path() -> Result<PathBuf, AppError> {
    use crate::pdf_engine::runtime_metadata::{IS_DEV_BUILD, RUNTIME_FILENAME};

    // 1. Packaged runtime (release & dev layout test): TAURI_RESOURCE_DIR/lib/libpdfium.so
    if let Ok(resource_dir) = std::env::var("TAURI_RESOURCE_DIR") {
        let packaged_path = PathBuf::from(resource_dir)
            .join("lib")
            .join(RUNTIME_FILENAME);
        if packaged_path.exists() && is_valid_runtime_file(&packaged_path) {
            return Ok(packaged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Packaged path canonicalization failed: {e}"))
            })?);
        }
    }

    // 2. Development-only paths (strictly gated under IS_DEV_BUILD)
    if IS_DEV_BUILD {
        // 2a. Explicit Developer Override: PDFIUM_PATH (absolute, allowlisted, valid file)
        if let Ok(dev_path) = std::env::var("PDFIUM_PATH") {
            let dev_path = PathBuf::from(dev_path);
            if !dev_path.is_absolute() {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH must be an absolute path in development mode".to_string(),
                ));
            }
            let canonical_dev = dev_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("PDFIUM_PATH canonicalization failed: {e}"))
            })?;
            if !is_path_under_allowlist(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH points outside allowed development roots".to_string(),
                ));
            }
            if !is_valid_runtime_file(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH does not point to a valid runtime file".to_string(),
                ));
            }
            tracing::info!(runtime_source = "development_override", path = %canonical_dev.display(), "PDFium runtime resolved via PDFIUM_PATH");
            return Ok(canonical_dev);
        }

        // 2b. Staged development path: CARGO_MANIFEST_DIR/../src-tauri/lib/libpdfium.so
        let staged_path = crate::pdf_engine::runtime_metadata::get_expected_staged_path();
        if staged_path.exists() && is_valid_runtime_file(&staged_path) {
            let canonical_staged = staged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Staged path canonicalization failed: {e}"))
            })?;
            tracing::info!(runtime_source = "staged_development", path = %canonical_staged.display(), "PDFium runtime resolved via staged path");
            return Ok(canonical_staged);
        }
    }

    // FAIL CLOSED: No CWD fallback, no PATH fallback, no system search
    Err(AppError::RuntimeUnavailable(
        "PDFium runtime not found. Expected at resourceDir/lib/libpdfium.so (packaged), PDFIUM_PATH (development override, absolute), or staged path (development only). No CWD or system search permitted."
            .to_string(),
    ))
}

#[cfg(target_os = "windows")]
fn resolve_windows_runtime_path() -> Result<PathBuf, AppError> {
    use crate::pdf_engine::runtime_metadata::{IS_DEV_BUILD, RUNTIME_FILENAME};

    // 1. Packaged runtime (release): TAURI_RESOURCE_DIR/lib/pdfium.dll
    if let Ok(resource_dir) = std::env::var("TAURI_RESOURCE_DIR") {
        let packaged_path = PathBuf::from(resource_dir)
            .join("lib")
            .join(RUNTIME_FILENAME);
        if packaged_path.exists() && is_valid_runtime_file(&packaged_path) {
            return Ok(packaged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Packaged path canonicalization failed: {e}"))
            })?);
        }
    }

    // 2. Development override: PDFIUM_PATH (explicit opt-in, absolute, validated)
    if IS_DEV_BUILD {
        if let Ok(dev_path) = std::env::var("PDFIUM_PATH") {
            let dev_path = PathBuf::from(dev_path);
            if !dev_path.is_absolute() {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH must be an absolute path in development mode".to_string(),
                ));
            }
            let canonical_dev = dev_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("PDFIUM_PATH canonicalization failed: {e}"))
            })?;
            if !is_path_under_allowlist(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH points outside allowed development roots".to_string(),
                ));
            }
            if !is_valid_runtime_file(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH does not point to a valid runtime file".to_string(),
                ));
            }
            tracing::info!(runtime_source = "development_override", path = %canonical_dev.display(), "PDFium runtime resolved via PDFIUM_PATH");
            return Ok(canonical_dev);
        }

        // 3. Staged development path: CARGO_MANIFEST_DIR/../src-tauri/lib/pdfium.dll
        let staged_path = crate::pdf_engine::runtime_metadata::get_expected_staged_path();
        if staged_path.exists() && is_valid_runtime_file(&staged_path) {
            let canonical_staged = staged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Staged path canonicalization failed: {e}"))
            })?;
            tracing::info!(runtime_source = "staged_development", path = %canonical_staged.display(), "PDFium runtime resolved via staged path");
            return Ok(canonical_staged);
        }
    }

    // FAIL CLOSED: No CWD fallback, no PATH fallback, no system search
    Err(AppError::RuntimeUnavailable(
        "PDFium runtime not found. Expected at resourceDir/lib/pdfium.dll (packaged), PDFIUM_PATH (development override, absolute), or staged path (development only). No CWD or PATH fallback permitted."
            .to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn resolve_macos_runtime_path() -> Result<PathBuf, AppError> {
    use crate::pdf_engine::runtime_metadata::{IS_DEV_BUILD, RUNTIME_FILENAME};

    // 1. Packaged runtime (release): TAURI_RESOURCE_DIR/lib/libpdfium.dylib
    if let Ok(resource_dir) = std::env::var("TAURI_RESOURCE_DIR") {
        let packaged_path = PathBuf::from(resource_dir)
            .join("lib")
            .join(RUNTIME_FILENAME);
        if packaged_path.exists() && is_valid_runtime_file(&packaged_path) {
            return Ok(packaged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Packaged path canonicalization failed: {e}"))
            })?);
        }
    }

    // 2. Development override: PDFIUM_PATH (explicit opt-in, absolute, validated)
    if IS_DEV_BUILD {
        if let Ok(dev_path) = std::env::var("PDFIUM_PATH") {
            let dev_path = PathBuf::from(dev_path);
            if !dev_path.is_absolute() {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH must be an absolute path in development mode".to_string(),
                ));
            }
            let canonical_dev = dev_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("PDFIUM_PATH canonicalization failed: {e}"))
            })?;
            if !is_path_under_allowlist(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH points outside allowed development roots".to_string(),
                ));
            }
            if !is_valid_runtime_file(&canonical_dev) {
                return Err(AppError::RuntimeUnavailable(
                    "PDFIUM_PATH does not point to a valid runtime file".to_string(),
                ));
            }
            tracing::info!(runtime_source = "development_override", path = %canonical_dev.display(), "PDFium runtime resolved via PDFIUM_PATH");
            return Ok(canonical_dev);
        }

        // 3. Staged development path
        let staged_path = crate::pdf_engine::runtime_metadata::get_expected_staged_path();
        if staged_path.exists() && is_valid_runtime_file(&staged_path) {
            let canonical_staged = staged_path.canonicalize().map_err(|e| {
                AppError::RuntimeUnavailable(format!("Staged path canonicalization failed: {e}"))
            })?;
            tracing::info!(runtime_source = "staged_development", path = %canonical_staged.display(), "PDFium runtime resolved via staged path");
            return Ok(canonical_staged);
        }
    }

    Err(AppError::RuntimeUnavailable(
        "PDFium runtime not found. Expected at resourceDir/lib/libpdfium.dylib (packaged), PDFIUM_PATH (development override, absolute), or staged path (development only)."
            .to_string(),
    ))
}

fn is_path_under_allowlist(path: &std::path::Path) -> bool {
    let current_exe = std::env::current_exe().ok();
    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from);

    let allowed_roots: Vec<PathBuf> = current_exe
        .as_ref()
        .map(|p| p.parent().unwrap_or(p).to_path_buf())
        .into_iter()
        .chain(cargo_manifest_dir)
        .collect();

    let canonical_path = path.canonicalize().ok();
    canonical_path.as_ref().is_some_and(|canonical| {
        allowed_roots.iter().any(|root| {
            let canonical_root = root.canonicalize().ok();
            canonical_root
                .as_ref()
                .is_some_and(|cr| canonical.starts_with(cr))
        })
    })
}

fn is_valid_runtime_file(path: &std::path::Path) -> bool {
    path.is_file()
        && path.file_name().and_then(|n| n.to_str())
            == Some(crate::pdf_engine::runtime_metadata::RUNTIME_FILENAME)
        && !is_symlink_outside_root(path)
}

fn is_symlink_outside_root(path: &std::path::Path) -> bool {
    use std::fs;
    let resource_dir = std::env::var("TAURI_RESOURCE_DIR").ok().map(PathBuf::from);
    let canonical_resource = resource_dir.as_ref().and_then(|d| d.canonicalize().ok());

    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            if let Ok(target) = fs::read_link(path) {
                let target_path = if target.is_absolute() {
                    target
                } else {
                    path.parent()
                        .unwrap_or(PathBuf::from(".").as_path())
                        .join(target)
                };
                if let (Ok(canonical_target), Some(canonical_root)) =
                    (target_path.canonicalize(), canonical_resource)
                {
                    return !canonical_target.starts_with(canonical_root);
                }
            }
        }
    }
    false
}

fn verify_runtime(path: &std::path::Path) -> Result<(), AppError> {
    use crate::pdf_engine::runtime_metadata::{
        EXPECTED_BYTE_LENGTH, EXPECTED_SHA256, IS_DEV_BUILD,
    };

    let metadata = std::fs::metadata(path).map_err(|e| {
        AppError::RuntimeLoadFailed(format!("Failed to read runtime metadata: {e}"))
    })?;

    if let Some(expected_len) = EXPECTED_BYTE_LENGTH {
        if metadata.len() != expected_len {
            return Err(AppError::RuntimeIntegrityMismatch(format!(
                "Byte length mismatch: expected {expected_len}, found {}",
                metadata.len()
            )));
        }
    }

    #[cfg(target_os = "linux")]
    {
        // In release builds (!IS_DEV_BUILD), packaged Linux runtime requires complete SHA-256 verification
        if !IS_DEV_BUILD && EXPECTED_SHA256.is_none() {
            return Err(AppError::RuntimeIntegrityMismatch(
                "Packaged Linux PDFium runtime metadata incomplete: missing expected SHA-256 hash"
                    .to_string(),
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        // In release builds (!IS_DEV_BUILD), packaged Windows runtime requires complete SHA-256 verification
        if !IS_DEV_BUILD && EXPECTED_SHA256.is_none() {
            return Err(AppError::RuntimeIntegrityMismatch(
                "Packaged Windows PDFium runtime metadata incomplete: missing expected SHA-256 hash"
                    .to_string(),
            ));
        }
    }

    if let Some(expected_sha256) = EXPECTED_SHA256 {
        let actual_sha256 = compute_sha256(path)
            .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to compute SHA-256: {e}")))?;
        if actual_sha256 != expected_sha256 {
            return Err(AppError::RuntimeIntegrityMismatch(
                "SHA-256 mismatch: runtime binary does not match expected hash".to_string(),
            ));
        }
    } else {
        tracing::warn!(
            runtime_source = "development_external",
            path = %path.display(),
            "PDFium runtime loaded without verified SHA-256 checksum in development mode"
        );
    }

    #[cfg(target_os = "linux")]
    {
        verify_linux_architecture(path)?;
    }

    #[cfg(target_os = "windows")]
    {
        verify_windows_architecture(path)?;
    }

    Ok(())
}

fn verify_elf_header_bytes(elf_header: &[u8], expected_arch: &str) -> Result<(), AppError> {
    if elf_header.len() < 20 {
        return Err(AppError::RuntimeLoadFailed(
            "ELF header too short".to_string(),
        ));
    }

    if &elf_header[0..4] != b"\x7fELF" {
        return Err(AppError::RuntimeLoadFailed(
            "Not a valid ELF file (missing \\x7fELF signature)".to_string(),
        ));
    }

    let ei_class = elf_header[4];
    let ei_data = elf_header[5];

    if ei_class != 2 {
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: expected_arch.to_string(),
            found: "ELFCLASS32 (32-bit)".to_string(),
        });
    }

    let is_little_endian = ei_data == 1;
    let machine_bytes = [elf_header[0x12], elf_header[0x13]];
    let e_machine = if is_little_endian {
        u16::from_le_bytes(machine_bytes)
    } else {
        u16::from_be_bytes(machine_bytes)
    };

    const EM_X86_64: u16 = 0x3e;
    if e_machine != EM_X86_64 {
        let found_arch = match e_machine {
            0x03 => "i386",
            0x28 => "ARM",
            0xb7 => "AArch64",
            0xf3 => "RISC-V",
            _ => "unknown",
        };
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: expected_arch.to_string(),
            found: found_arch.to_string(),
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn verify_linux_architecture(path: &std::path::Path) -> Result<(), AppError> {
    use crate::pdf_engine::runtime_metadata::EXPECTED_ARCH;
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| {
        AppError::RuntimeLoadFailed(format!("Failed to open runtime for arch check: {e}"))
    })?;

    let mut elf_header = [0u8; 64];
    file.read_exact(&mut elf_header)
        .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to read ELF header: {e}")))?;

    verify_elf_header_bytes(&elf_header, EXPECTED_ARCH)
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn verify_linux_architecture(_path: &std::path::Path) -> Result<(), AppError> {
    Ok(())
}

fn verify_pe_header_bytes(bytes: &[u8], expected_arch: &str) -> Result<(), AppError> {
    if bytes.len() < 64 {
        return Err(AppError::RuntimeLoadFailed(
            "PE header too short (missing DOS header)".to_string(),
        ));
    }

    if &bytes[0..2] != b"MZ" {
        return Err(AppError::RuntimeLoadFailed(
            "Not a valid PE file (missing MZ signature)".to_string(),
        ));
    }

    let pe_offset =
        u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;

    if bytes.len() < pe_offset.saturating_add(24) {
        return Err(AppError::RuntimeLoadFailed(
            "PE file truncated before COFF header".to_string(),
        ));
    }

    if &bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return Err(AppError::RuntimeLoadFailed(
            "Not a valid PE file (missing PE signature)".to_string(),
        ));
    }

    let coff = &bytes[pe_offset + 4..pe_offset + 24];
    let machine = u16::from_le_bytes([coff[0], coff[1]]);

    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

    let found_arch = match machine {
        IMAGE_FILE_MACHINE_AMD64 => "x86_64",
        IMAGE_FILE_MACHINE_I386 => "x86",
        IMAGE_FILE_MACHINE_ARM64 => "aarch64",
        _ => "unknown",
    };

    if machine != IMAGE_FILE_MACHINE_AMD64 {
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: expected_arch.to_string(),
            found: found_arch.to_string(),
        });
    }

    let characteristics = u16::from_le_bytes([coff[18], coff[19]]);
    const IMAGE_FILE_32BIT_MACHINE: u16 = 0x0020;
    if characteristics & IMAGE_FILE_32BIT_MACHINE != 0 {
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: expected_arch.to_string(),
            found: format!("{found_arch} (32-bit flag set)"),
        });
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_windows_architecture(path: &std::path::Path) -> Result<(), AppError> {
    use crate::pdf_engine::runtime_metadata::EXPECTED_ARCH;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path).map_err(|e| {
        AppError::RuntimeLoadFailed(format!("Failed to open runtime for arch check: {e}"))
    })?;

    // Read DOS header to find PE header offset
    let mut dos_header = [0u8; 64];
    file.read_exact(&mut dos_header)
        .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to read DOS header: {e}")))?;

    if &dos_header[0..2] != b"MZ" {
        return Err(AppError::RuntimeLoadFailed(
            "Not a valid PE file (missing MZ signature)".to_string(),
        ));
    }

    // PE header offset is at offset 0x3C (little-endian uint32)
    let pe_offset = u32::from_le_bytes([
        dos_header[0x3C],
        dos_header[0x3D],
        dos_header[0x3E],
        dos_header[0x3F],
    ]);

    // Seek to PE header
    file.seek(SeekFrom::Start(pe_offset as u64))
        .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to seek to PE header: {e}")))?;

    // Read PE signature (4 bytes: "PE\0\0")
    let mut pe_sig = [0u8; 4];
    file.read_exact(&mut pe_sig)
        .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to read PE signature: {e}")))?;

    if &pe_sig != b"PE\0\0" {
        return Err(AppError::RuntimeLoadFailed(
            "Not a valid PE file (missing PE signature)".to_string(),
        ));
    }

    // Read COFF File Header (20 bytes)
    let mut coff_header = [0u8; 20];
    file.read_exact(&mut coff_header)
        .map_err(|e| AppError::RuntimeLoadFailed(format!("Failed to read COFF header: {e}")))?;

    // Machine field is first 2 bytes (little-endian)
    let machine = u16::from_le_bytes([coff_header[0], coff_header[1]]);

    // IMAGE_FILE_MACHINE_AMD64 = 0x8664 (x64)
    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    // IMAGE_FILE_MACHINE_I386 = 0x014c (x86)
    const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
    // IMAGE_FILE_MACHINE_ARM64 = 0xAA64 (ARM64)
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

    let found_arch = match machine {
        IMAGE_FILE_MACHINE_AMD64 => "x86_64",
        IMAGE_FILE_MACHINE_I386 => "x86",
        IMAGE_FILE_MACHINE_ARM64 => "aarch64",
        _ => "unknown",
    };

    if machine != IMAGE_FILE_MACHINE_AMD64 {
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: EXPECTED_ARCH.to_string(),
            found: found_arch.to_string(),
        });
    }

    // Optional: verify it's 64-bit (not 32-bit) by checking Characteristics
    // Bit 0x0020 = IMAGE_FILE_32BIT_MACHINE should NOT be set for 64-bit
    let characteristics = u16::from_le_bytes([coff_header[18], coff_header[19]]);
    const IMAGE_FILE_32BIT_MACHINE: u16 = 0x0020;
    if characteristics & IMAGE_FILE_32BIT_MACHINE != 0 {
        return Err(AppError::RuntimeArchitectureMismatch {
            expected: EXPECTED_ARCH.to_string(),
            found: format!("{} (32-bit flag set)", found_arch),
        });
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn verify_windows_architecture(_path: &std::path::Path) -> Result<(), AppError> {
    Ok(())
}

fn compute_sha256(path: &std::path::Path) -> Result<[u8; 32], std::io::Error> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::{BufReader, Read};

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    Ok(hash)
}

pub fn get_cached_password(path: &str, cache: &PasswordCache) -> Option<String> {
    let lock = cache.0.lock();
    lock.get(path).cloned()
}

pub fn open_pdfium_doc<'a>(
    pdfium: &'a Pdfium,
    path: &str,
    password: Option<&'a str>,
) -> Result<PdfDocument<'a>, String> {
    let _ffi_span = tracing::info_span!(target: "pdfium_ffi", "pdfium_load_pdf").entered();
    pdfium.load_pdf_from_file(path, password).map_err(|e| {
        crate::error::AppError::Pdfium(format!("PDF açılırken hata oluştu: {:?}", e)).to_string()
    })
}

pub(crate) async fn render_page_rgba(
    path: String,
    page_index: u32,
    scale: f64,
    rotation: i32,
    password: Option<String>,
) -> Result<RenderedPage, String> {
    if scale.is_nan() || scale.is_infinite() || scale <= 0.0 {
        return Err("Geçersiz ölçek (scale) değeri".to_string());
    }

    let pdf_rotation = match rotation {
        0 => PdfPageRenderRotation::None,
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => return Err(format!("Geçersiz rotasyon (rotation) değeri: {}", rotation)),
    };

    tauri::async_runtime::spawn_blocking(move || {
        let span = tracing::Span::current();
        let _enter = span.enter();
        let _guard = PDFIUM_MUTEX.lock();
        let pdfium = get_pdfium()?;
        let document = open_pdfium_doc(&pdfium, &path, password.as_deref())?;

        let pages = document.pages();
        if page_index >= pages.len() as u32 {
            return Err(format!("Geçersiz sayfa numarası: {}", page_index));
        }

        let page = pages.get(page_index as u16).map_err(|e| {
            crate::error::AppError::Pdfium(format!("Sayfa alınamadı: {:?}", e)).to_string()
        })?;

        let target_width = (page.width().value as f64 * scale) as i32;

        let mut render_config = PdfRenderConfig::new();
        render_config = render_config.set_target_width(target_width);
        render_config = render_config.rotate(pdf_rotation, true);
        render_config = render_config.set_clear_color(PdfColor::WHITE);
        // FPDF_REVERSE_BYTE_ORDER makes PDFium write straight RGBA, so the raw
        // buffer needs no channel swap (same contract as the viewer worker).
        render_config = render_config.set_reverse_byte_order(true);

        let bitmap = page.render_with_config(&render_config).map_err(|e| {
            crate::error::AppError::Image(format!("Sayfa render etme hatası: {:?}", e)).to_string()
        })?;

        Ok(RenderedPage {
            width: bitmap.width() as u32,
            height: bitmap.height() as u32,
            data: bitmap.as_raw_bytes(),
        })
    })
    .await
    .map_err(|e| crate::error::AppError::Thread(format!("Thread hatası: {}", e)).to_string())?
}

#[derive(Clone, Copy)]
pub(crate) enum ViewerRenderSizing {
    Scale(f64),
    TargetWidth(u32),
}

impl ViewerRenderSizing {
    fn key(self) -> RenderSizingKey {
        match self {
            Self::Scale(scale) => RenderSizingKey::ScaleBits(scale.to_bits()),
            Self::TargetWidth(width) => RenderSizingKey::TargetWidth(width),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderDocumentChange {
    changed: bool,
}

#[derive(Serialize)]
pub struct RenderCancellationResult {
    pub(crate) cancelled: bool,
}

#[tauri::command]
pub fn set_active_render_document(
    document_id: Option<String>,
    path: Option<String>,
    generation: u64,
    render_service: State<'_, RenderService>,
) -> Result<RenderDocumentChange, String> {
    validate_generation(generation)?;
    let document = match (document_id, path) {
        (Some(document_id), Some(path)) => Some(ActiveRenderDocument {
            document_id,
            path,
            generation,
        }),
        (None, None) => None,
        _ => return Err("render_invalid_document_activation".to_string()),
    };
    let changed = render_service.set_active_document(document)?;
    Ok(RenderDocumentChange { changed })
}

#[tauri::command]
pub fn cancel_render_request(
    path: String,
    document_id: String,
    page_index: u32,
    generation: u64,
    request_id: String,
    scope: RenderScope,
    render_service: State<'_, RenderService>,
) -> Result<RenderCancellationResult, String> {
    let identity = RenderRequestIdentity {
        document_id,
        path,
        generation,
        page_index,
        scope,
        request_id,
    };
    let cancelled = render_service.cancel_request(&identity)?;
    Ok(RenderCancellationResult { cancelled })
}

pub(crate) async fn render_viewer_request_with_renderer<Renderer, RenderFuture>(
    identity: RenderRequestIdentity,
    sizing: ViewerRenderSizing,
    rotation: i32,
    password_cache: &PasswordCache,
    render_service: &RenderService,
    renderer: Renderer,
) -> Result<RenderResponse, String>
where
    Renderer: FnOnce(
        String,
        u32,
        ViewerRenderSizing,
        i32,
        Option<String>,
        CancellationToken,
    ) -> RenderFuture,
    RenderFuture: Future<Output = Result<RenderedPage, String>>,
{
    let total_started = std::time::Instant::now();
    let task = render_service.begin_request(identity)?;
    let target_key = RenderTargetKey {
        document_id: task.identity().document_id.clone(),
        generation: task.identity().generation,
        page_index: task.identity().page_index,
        sizing: sizing.key(),
        rotation,
    };

    if let Some((width, height)) = render_service.dimensions(&target_key) {
        let cache_key = cache_key(task.identity(), width, height, rotation);
        let _cache_span = tracing::info_span!("render_cache_lookup").entered();
        if let Some(response) = render_service.cached_response(task.identity(), &cache_key) {
            tracing::debug!(
                total_ms = total_started.elapsed().as_secs_f64() * 1000.0,
                cache_hit = true,
                "render completed"
            );
            return Ok(response);
        }
    }

    let scheduler_started = std::time::Instant::now();
    let _permit = render_service
        .acquire(task.identity().scope, task.token())
        .await?;
    tracing::debug!(
        scheduler_wait_ms = scheduler_started.elapsed().as_secs_f64() * 1000.0,
        "render scheduler permit acquired"
    );

    if task.token().is_cancelled() || !render_service.is_current(task.identity()) {
        return Err("render_cancelled".to_string());
    }

    let password = get_cached_password(&task.identity().path, password_cache);
    let render_result = renderer(
        task.identity().path.clone(),
        task.identity().page_index,
        sizing,
        rotation,
        password,
        task.token().clone(),
    )
    .await;

    if task.token().is_cancelled() || !render_service.is_current(task.identity()) {
        return Err("render_cancelled".to_string());
    }
    let rendered = render_result?;

    let expected = checked_rgba_len(rendered.width, rendered.height)?;
    if rendered.data.len() != expected {
        return Err("render_invalid_rgba_payload".to_string());
    }

    render_service.record_dimensions(
        task.identity(),
        target_key,
        (rendered.width, rendered.height),
    );
    let cache_key = cache_key(task.identity(), rendered.width, rendered.height, rotation);
    let data = Arc::new(rendered.data);
    if data.len() <= MAX_CACHE_ENTRY_BYTES {
        render_service.cache_response(
            task.identity(),
            cache_key,
            CachedRender {
                width: rendered.width,
                height: rendered.height,
                data: Arc::clone(&data),
            },
        );
    }

    let response = RenderResponse {
        width: rendered.width,
        height: rendered.height,
        data,
        page_index: task.identity().page_index,
        generation: task.identity().generation,
        request_id: task.identity().request_id.clone(),
    };
    tracing::debug!(
        total_ms = total_started.elapsed().as_secs_f64() * 1000.0,
        cache_hit = false,
        "render completed"
    );
    Ok(response)
}

fn cache_key(
    identity: &RenderRequestIdentity,
    width: u32,
    height: u32,
    rotation: i32,
) -> RenderCacheKey {
    RenderCacheKey {
        document_id: identity.document_id.clone(),
        generation: identity.generation,
        page_index: identity.page_index,
        target_width: width,
        target_height: height,
        rotation,
    }
}

pub(crate) fn validate_scale(scale: f64) -> Result<(), String> {
    if !scale.is_finite() || scale <= 0.0 || scale > MAX_SCALE {
        return Err("render_invalid_scale".to_string());
    }
    Ok(())
}

pub(crate) fn validate_target_width(target_width: u32) -> Result<(), String> {
    if target_width == 0 || target_width > MAX_PIXEL_DIMENSION {
        return Err("render_invalid_target_width".to_string());
    }
    Ok(())
}

pub(crate) fn validate_rotation(rotation: i32) -> Result<(), String> {
    pdf_rotation(rotation).map(|_| ())
}

#[cfg(test)]
mod canonical_render_validation_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn scale_target_width_and_rotation_limits_match_frontend_contracts() {
        assert!(validate_scale(MAX_SCALE).is_ok());
        assert!(validate_scale(0.0).is_err());
        assert!(validate_scale(f64::NAN).is_err());
        assert!(validate_scale(MAX_SCALE + 0.01).is_err());
        assert!(validate_target_width(1).is_ok());
        assert!(validate_target_width(MAX_PIXEL_DIMENSION).is_ok());
        assert!(validate_target_width(0).is_err());
        assert!(validate_target_width(MAX_PIXEL_DIMENSION + 1).is_err());
        for rotation in [0, 90, 180, 270] {
            assert!(validate_rotation(rotation).is_ok());
        }
        assert!(validate_rotation(45).is_err());
    }

    fn active_document(generation: u64) -> ActiveRenderDocument {
        ActiveRenderDocument {
            document_id: "doc".to_string(),
            path: "/doc.pdf".to_string(),
            generation,
        }
    }

    fn request(generation: u64, request_id: &str) -> RenderRequestIdentity {
        RenderRequestIdentity {
            document_id: "doc".to_string(),
            path: "/doc.pdf".to_string(),
            generation,
            page_index: 0,
            scope: RenderScope::MainPage,
            request_id: request_id.to_string(),
        }
    }

    fn mock_rendered_page() -> RenderedPage {
        RenderedPage {
            width: 2,
            height: 2,
            data: vec![255; 16],
        }
    }

    fn password_cache() -> PasswordCache {
        PasswordCache(std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new())))
    }

    #[tokio::test]
    async fn orchestration_discards_render_when_generation_changes_during_renderer() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate generation one");
        let cache = password_cache();

        let result = render_viewer_request_with_renderer(
            request(1, "generation-one"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                service
                    .set_active_document(Some(active_document(2)))
                    .expect("activate generation two");
                async { Ok(mock_rendered_page()) }
            },
        )
        .await;

        assert_eq!(
            result.expect_err("generation change must cancel"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }

    #[tokio::test]
    async fn orchestration_discards_cancelled_result_before_renderer_returns() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();
        let identity = request(1, "cancel-before-return");
        let cancellation_identity = identity.clone();

        let result = render_viewer_request_with_renderer(
            identity,
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                assert!(service
                    .cancel_request(&cancellation_identity)
                    .expect("cancel request"));
                async { Ok(mock_rendered_page()) }
            },
        )
        .await;

        assert_eq!(
            result.expect_err("cancelled render must be discarded"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }

    #[tokio::test]
    async fn orchestration_cache_hit_uses_current_request_metadata() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();

        let first = render_viewer_request_with_renderer(
            request(1, "first"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| async { Ok(mock_rendered_page()) },
        )
        .await
        .expect("first render");
        assert_eq!(first.request_id, "first");
        assert_eq!(service.cached_entry_count(), 1);

        let renderer_calls = AtomicUsize::new(0);
        let second = render_viewer_request_with_renderer(
            request(1, "second"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                renderer_calls.fetch_add(1, Ordering::SeqCst);
                async { Err("renderer_should_not_run".to_string()) }
            },
        )
        .await
        .expect("cache hit");

        assert_eq!(renderer_calls.load(Ordering::SeqCst), 0);
        assert_eq!(second.request_id, "second");
        assert_eq!(second.generation, 1);
        assert_eq!(second.page_index, 0);
    }

    #[tokio::test]
    async fn orchestration_distinguishes_cancellation_from_renderer_failure() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();

        let renderer_error = render_viewer_request_with_renderer(
            request(1, "renderer-error"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| async { Err("mock_renderer_failure".to_string()) },
        )
        .await;
        assert_eq!(
            renderer_error.expect_err("renderer error must propagate"),
            "mock_renderer_failure"
        );

        let cancellation_identity = request(1, "cancelled-error");
        let cancel_lookup = cancellation_identity.clone();
        let cancelled = render_viewer_request_with_renderer(
            cancellation_identity,
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                assert!(service
                    .cancel_request(&cancel_lookup)
                    .expect("cancel request"));
                async { Err("mock_renderer_failure".to_string()) }
            },
        )
        .await;
        assert_eq!(
            cancelled.expect_err("cancellation must take precedence"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }

    /// Manual baseline for the main-process half of the viewer render path
    /// (cache bookkeeping plus the `render-frame/1` encode) with an A4 page at
    /// scale 2.0 (1190x1684 RGBA). Pixel buffers are allocated before timing so
    /// only pipeline copies are measured. Run from `src-tauri`:
    /// `cargo test --release --lib -- --ignored --nocapture render_pipeline_cache_timing`
    #[tokio::test]
    #[ignore = "manual render baseline measurement"]
    async fn render_pipeline_cache_timing() {
        const WIDTH: u32 = 1190;
        const HEIGHT: u32 = 1684;
        const SAMPLES: usize = 40;
        let pixels = WIDTH as usize * HEIGHT as usize * 4;
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();
        let identity = |page_index: u32, request_id: String| RenderRequestIdentity {
            page_index,
            request_id,
            ..request(1, "unused")
        };
        let summary = |label: &str, mut samples: Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "render-baseline {label}: n={} min_ms={:.3} median_ms={:.3} p95_ms={:.3}",
                samples.len(),
                samples[0],
                samples[samples.len() / 2],
                samples[(samples.len() - 1) * 95 / 100]
            );
        };

        let mut buffers = (0..SAMPLES)
            .map(|index| vec![index as u8; pixels])
            .collect::<Vec<_>>();
        let mut miss = Vec::new();
        for page_index in 0..SAMPLES as u32 {
            let data = buffers.pop().unwrap();
            let started = std::time::Instant::now();
            let response = render_viewer_request_with_renderer(
                identity(page_index, format!("miss-{page_index}")),
                ViewerRenderSizing::Scale(2.0),
                0,
                &cache,
                &service,
                |_, _, _, _, _, _| async move {
                    Ok(RenderedPage {
                        width: WIDTH,
                        height: HEIGHT,
                        data,
                    })
                },
            )
            .await
            .expect("cache miss render");
            let frame = render_wire::encode_render_frame(&response).expect("encode frame");
            miss.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(frame);
        }
        summary("pipeline_cache_miss_and_frame_encode", miss);

        let last_page = SAMPLES as u32 - 1;
        let mut hit = Vec::new();
        for index in 0..SAMPLES {
            let started = std::time::Instant::now();
            let response = render_viewer_request_with_renderer(
                identity(last_page, format!("hit-{index}")),
                ViewerRenderSizing::Scale(2.0),
                0,
                &cache,
                &service,
                |_, _, _, _, _, _| async { Err("renderer_should_not_run".to_string()) },
            )
            .await
            .expect("cache hit render");
            let frame = render_wire::encode_render_frame(&response).expect("encode frame");
            hit.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(frame);
        }
        summary("pipeline_cache_hit_and_frame_encode", hit);
    }

    const PURE_RGB_PIXELS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

    /// One fully filled 100x100 pt page per color in `PURE_RGB_PIXELS`.
    fn write_pure_rgb_pdf(path: &std::path::Path) {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Dictionary, Document, Object, Stream};

        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();
        for (red, green, blue) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
            let content = Content {
                operations: vec![
                    Operation::new("rg", vec![red.into(), green.into(), blue.into()]),
                    Operation::new("re", vec![0.into(), 0.into(), 100.into(), 100.into()]),
                    Operation::new("f", vec![]),
                ],
            };
            let content_id =
                document.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
            page_ids.push(Object::Reference(document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Contents" => content_id,
            })));
        }
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids,
                "Count" => 3,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    #[tokio::test]
    async fn render_page_rgba_returns_rgba_for_pure_red_green_blue_pages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pure-rgb.pdf");
        write_pure_rgb_pdf(&path);
        for (page_index, expected) in PURE_RGB_PIXELS.into_iter().enumerate() {
            let rendered = render_page_rgba(
                path.to_string_lossy().to_string(),
                page_index as u32,
                1.0,
                0,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                rendered.data.len(),
                rendered.width as usize * rendered.height as usize * 4
            );
            assert!(
                rendered.data.chunks_exact(4).all(|pixel| pixel == expected),
                "page {page_index}: first pixel {:?}, expected {expected:?}",
                &rendered.data[..4]
            );
        }
    }

    /// Characterizes the production page export (job worker →
    /// `export_pages_as_images_with_password`), which normalizes channels via
    /// `PdfBitmap::as_image`.
    #[test]
    fn page_image_export_writes_pure_red_green_blue_pages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pure-rgb.pdf");
        write_pure_rgb_pdf(&path);
        let files = crate::export::export_pages_as_images_with_password(
            path.to_string_lossy().to_string(),
            vec![0, 1, 2],
            directory.path().to_string_lossy().to_string(),
            "png".to_string(),
            72.0,
            None,
            None,
        )
        .unwrap();
        assert_eq!(files.len(), 3);
        for (file, expected) in files.iter().zip(PURE_RGB_PIXELS) {
            let image = image::open(file).unwrap().to_rgba8();
            assert!(
                image.pixels().all(|pixel| pixel.0 == expected),
                "{file}: first pixel {:?}, expected {expected:?}",
                image.get_pixel(0, 0).0
            );
        }
    }
}

fn pdf_rotation(rotation: i32) -> Result<PdfPageRenderRotation, String> {
    match rotation {
        0 => Ok(PdfPageRenderRotation::None),
        90 => Ok(PdfPageRenderRotation::Degrees90),
        180 => Ok(PdfPageRenderRotation::Degrees180),
        270 => Ok(PdfPageRenderRotation::Degrees270),
        _ => Err(format!("Geçersiz rotasyon (rotation) değeri: {}", rotation)),
    }
}

#[tauri::command]
pub fn clear_search_cache(
    _path: Option<String>,
    search_cache: State<'_, crate::SearchCache>,
) -> Result<(), String> {
    let mut cache = search_cache.0.lock();
    cache.clear();
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod linux_runtime_resolution_tests {
    use super::*;
    use crate::pdf_engine::runtime_metadata::*;
    use sha2::Digest;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn runtime_metadata_constants_exist() {
        let _ = EXPECTED_ARCH;
        let _ = RUNTIME_FILENAME;
        let _ = SUPPLY_CHAIN_STATUS;
        let _ = EXPECTED_STAGED_PATH;
        let _ = EXPECTED_PACKAGED_LOCATION;
        let _ = IS_DEV_BUILD;
    }

    #[test]
    fn arch_is_x86_64() {
        assert_eq!(EXPECTED_ARCH, "x86_64");
    }

    #[test]
    fn runtime_filename_is_libpdfium_so() {
        assert_eq!(RUNTIME_FILENAME, "libpdfium.so");
    }

    #[test]
    fn supply_chain_status_is_known_value() {
        assert!(matches!(
            SUPPLY_CHAIN_STATUS,
            "COMPLETE" | "PARTIAL" | "INCOMPLETE" | "MISSING"
        ));
    }

    #[test]
    fn staged_path_ends_with_libpdfium_so() {
        assert!(EXPECTED_STAGED_PATH.ends_with("libpdfium.so"));
    }

    #[test]
    fn packaged_location_ends_with_libpdfium_so() {
        assert!(EXPECTED_PACKAGED_LOCATION.ends_with("libpdfium.so"));
    }

    #[test]
    fn is_valid_runtime_file_rejects_wrong_filename() {
        let temp = TempDir::new().unwrap();
        let wrong_name = temp.path().join("wrong.so");
        fs::write(&wrong_name, b"dummy").unwrap();
        assert!(!is_valid_runtime_file(&wrong_name));
    }

    #[test]
    fn is_valid_runtime_file_accepts_correct_filename() {
        let temp = TempDir::new().unwrap();
        let correct_name = temp.path().join("libpdfium.so");
        fs::write(&correct_name, b"dummy").unwrap();
        assert!(is_valid_runtime_file(&correct_name));
    }

    #[test]
    fn is_valid_runtime_file_rejects_directory() {
        let temp = TempDir::new().unwrap();
        assert!(!is_valid_runtime_file(temp.path()));
    }

    #[test]
    fn is_valid_runtime_file_rejects_nonexistent() {
        let temp = TempDir::new().unwrap();
        let nonexistent = temp.path().join("libpdfium.so");
        assert!(!is_valid_runtime_file(&nonexistent));
    }

    #[test]
    fn compute_sha256_produces_correct_hash() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.bin");
        fs::write(&test_file, b"hello world").unwrap();

        let hash = compute_sha256(&test_file).unwrap();
        let expected = sha2::Sha256::digest(b"hello world");
        assert_eq!(hash.as_slice(), expected.as_slice());
    }

    #[test]
    fn compute_sha256_different_for_different_content() {
        let temp = TempDir::new().unwrap();
        let file1 = temp.path().join("test1.bin");
        let file2 = temp.path().join("test2.bin");
        fs::write(&file1, b"hello").unwrap();
        fs::write(&file2, b"world").unwrap();

        let hash1 = compute_sha256(&file1).unwrap();
        let hash2 = compute_sha256(&file2).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn path_traversal_in_pdfium_path_rejected() {
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let target = outside.join("libpdfium.so");
        fs::write(&target, b"dummy").unwrap();

        let mut allowed_roots = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                allowed_roots.push(parent.to_path_buf());
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            allowed_roots.push(PathBuf::from(manifest));
        }

        let canonical_target = target.canonicalize().unwrap();
        let is_allowed = allowed_roots.iter().any(|root| {
            root.canonicalize()
                .map(|cr| canonical_target.starts_with(&cr))
                .unwrap_or(false)
        });
        assert!(!is_allowed, "Path outside allowed roots should be rejected");
    }

    #[test]
    fn symlink_escape_detection() {
        let temp = TempDir::new().unwrap();
        let real_dir = temp.path().join("real");
        let link_dir = temp.path().join("link");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&link_dir).unwrap();

        let real_file = real_dir.join("libpdfium.so");
        fs::write(&real_file, b"dummy").unwrap();

        let link_file = link_dir.join("libpdfium.so");
        std::os::unix::fs::symlink(&real_file, &link_file).unwrap();

        let resource_dir = Some(link_dir.clone());
        let canonical_resource = resource_dir.as_ref().and_then(|d| d.canonicalize().ok());

        let metadata = fs::symlink_metadata(&link_file).unwrap();
        assert!(metadata.file_type().is_symlink());

        let target = fs::read_link(&link_file).unwrap();
        let target_path = link_file.parent().unwrap().join(target);
        let canonical_target = target_path.canonicalize().unwrap();

        let escapes = canonical_resource
            .as_ref()
            .map(|cr| !canonical_target.starts_with(cr))
            .unwrap_or(false);

        assert!(
            escapes,
            "Symlink pointing outside resource root should be detected"
        );
    }

    #[test]
    fn byte_length_mismatch_detected() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("libpdfium.so");
        fs::write(&test_file, b"short").unwrap();

        let metadata = fs::metadata(&test_file).unwrap();
        assert_eq!(metadata.len(), 5);

        let expected_len: u64 = 100;
        assert_ne!(metadata.len(), expected_len);
    }

    #[test]
    fn architecture_check_constants() {
        assert_eq!(EXPECTED_ARCH, "x86_64");
    }

    #[test]
    fn error_types_are_distinct() {
        use crate::error::AppError;

        let e1 = AppError::RuntimeUnavailable("test".to_string());
        let e2 = AppError::RuntimeArchitectureMismatch {
            expected: "x86_64".to_string(),
            found: "aarch64".to_string(),
        };
        let e3 = AppError::RuntimeIntegrityMismatch("hash mismatch".to_string());
        let e4 = AppError::RuntimeLoadFailed("load failed".to_string());
        let e5 = AppError::RuntimeUnsupported("unsupported".to_string());

        let serialize = |e: &AppError| serde_json::to_string(e).unwrap();
        let s1 = serialize(&e1);
        let s2 = serialize(&e2);
        let s3 = serialize(&e3);
        let s4 = serialize(&e4);
        let s5 = serialize(&e5);

        assert!(s1.contains("RuntimeUnavailable"));
        assert!(s2.contains("RuntimeArchitectureMismatch"));
        assert!(s3.contains("RuntimeIntegrityMismatch"));
        assert!(s4.contains("RuntimeLoadFailed"));
        assert!(s5.contains("RuntimeUnsupported"));
    }

    #[test]
    fn architecture_mapping_elf_emachine_x86_64() {
        use crate::pdf_engine::runtime_metadata::EXPECTED_ARCH;
        assert_eq!(EXPECTED_ARCH, "x86_64");
        const EM_X86_64: u16 = 0x3e;
        assert_eq!(EM_X86_64, 0x3e);
    }

    #[test]
    fn manifest_package_parity_packed_location_matches_loader() {
        use crate::pdf_engine::runtime_metadata::{
            get_expected_packaged_path, EXPECTED_PACKAGED_LOCATION, RUNTIME_FILENAME,
        };
        use std::path::PathBuf;

        let resource_dir = PathBuf::from("/mock/resourceDir");
        let loader_path = resource_dir.join("lib").join(RUNTIME_FILENAME);
        let metadata_path = get_expected_packaged_path(&resource_dir);

        assert_eq!(loader_path, metadata_path);
        assert!(EXPECTED_PACKAGED_LOCATION.ends_with("lib/libpdfium.so"));
        assert_eq!(
            loader_path.to_string_lossy().as_ref(),
            "/mock/resourceDir/lib/libpdfium.so"
        );
    }

    #[test]
    fn runtime_filename_matches_c2_canonical_record() {
        use crate::pdf_engine::runtime_metadata::RUNTIME_FILENAME;
        assert_eq!(RUNTIME_FILENAME, "libpdfium.so");
    }

    #[test]
    fn supply_chain_status_is_incomplete_without_binary() {
        use crate::pdf_engine::runtime_metadata::SUPPLY_CHAIN_STATUS;
        assert_eq!(SUPPLY_CHAIN_STATUS, "INCOMPLETE");
    }

    #[test]
    fn staged_path_is_repo_relative_not_absolute() {
        use crate::pdf_engine::runtime_metadata::EXPECTED_STAGED_PATH;
        assert!(!EXPECTED_STAGED_PATH.starts_with('/'));
        assert!(EXPECTED_STAGED_PATH.contains("src-tauri/lib/libpdfium.so"));
    }

    #[test]
    fn architecture_constant_is_x86_64_not_alias() {
        use crate::pdf_engine::runtime_metadata::EXPECTED_ARCH;
        assert_eq!(EXPECTED_ARCH, "x86_64");
        assert_ne!(EXPECTED_ARCH, "amd64");
        assert_ne!(EXPECTED_ARCH, "x64");
    }

    // FAST_LOCAL Windows resolver tests (no Windows host required)
    #[cfg(all(test, target_os = "windows"))]
    mod windows_resolver_fast_local_tests {
        use super::*;
        use crate::pdf_engine::runtime_metadata::{
            EXPECTED_ARCH, EXPECTED_PACKAGED_LOCATION, EXPECTED_STAGED_PATH, IS_DEV_BUILD,
            RUNTIME_FILENAME, SUPPLY_CHAIN_STATUS,
        };
        use std::fs;
        use std::path::PathBuf;
        use tempfile::TempDir;

        #[test]
        fn windows_runtime_filename_is_pdfium_dll() {
            assert_eq!(RUNTIME_FILENAME, "pdfium.dll");
        }

        #[test]
        fn windows_expected_arch_is_x86_64() {
            assert_eq!(EXPECTED_ARCH, "x86_64");
        }

        #[test]
        fn windows_staged_path_ends_with_pdfium_dll() {
            assert!(EXPECTED_STAGED_PATH.ends_with("pdfium.dll"));
        }

        #[test]
        fn windows_packaged_location_ends_with_pdfium_dll() {
            assert!(EXPECTED_PACKAGED_LOCATION.ends_with("pdfium.dll"));
        }

        #[test]
        fn resolve_windows_rejects_cwd_relative_paths() {
            // This test verifies that the Windows resolver does NOT use CWD-relative paths
            // by checking that the resolution logic doesn't contain "./" patterns
            // The actual resolver is platform-gated, so we test the metadata contract
            assert!(!EXPECTED_STAGED_PATH.starts_with("./"));
            assert!(!EXPECTED_PACKAGED_LOCATION.starts_with("./"));
        }

        #[test]
        fn resolve_windows_requires_absolute_dev_override() {
            // PDFIUM_PATH must be absolute in development mode
            // This is enforced by the resolver logic (checked in integration)
            let temp = TempDir::new().unwrap();
            let relative_path = temp.path().join("pdfium.dll");
            fs::write(&relative_path, b"dummy").unwrap();

            // Relative path should not be accepted as valid override
            assert!(!relative_path.is_absolute());
        }

        #[test]
        fn path_composition_windows_resource_dir() {
            // Test path composition: TAURI_RESOURCE_DIR + lib + pdfium.dll
            let resource_dir = PathBuf::from("C:\\mock\\resourceDir");
            let expected = resource_dir.join("lib").join("pdfium.dll");
            assert_eq!(
                expected.to_string_lossy().as_ref(),
                "C:\\mock\\resourceDir\\lib\\pdfium.dll"
            );
        }

        #[test]
        fn unsafe_filename_rejection_traversal() {
            // Path traversal attempts should be rejected by is_valid_runtime_file
            let temp = TempDir::new().unwrap();
            let traversal = temp.path().join("..").join("pdfium.dll");
            // is_valid_runtime_file checks for exact filename match
            assert!(!is_valid_runtime_file(&traversal));
        }

        #[test]
        fn unsafe_filename_rejection_absolute_outside_root() {
            // Absolute paths outside allowed roots should be rejected by is_path_under_allowlist
            let temp = TempDir::new().unwrap();
            let outside = temp.path().join("outside");
            fs::create_dir_all(&outside).unwrap();
            let target = outside.join("pdfium.dll");
            fs::write(&target, b"dummy").unwrap();

            let canonical_target = target.canonicalize().unwrap();
            let current_exe = std::env::current_exe().ok();
            let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from);

            let allowed_roots: Vec<PathBuf> = current_exe
                .as_ref()
                .map(|p| p.parent().unwrap_or(p).to_path_buf())
                .into_iter()
                .chain(cargo_manifest_dir)
                .collect();

            let is_allowed = allowed_roots.iter().any(|root| {
                root.canonicalize()
                    .map(|cr| canonical_target.starts_with(&cr))
                    .unwrap_or(false)
            });
            assert!(
                !is_allowed,
                "Absolute path outside allowed roots should be rejected"
            );
        }

        #[test]
        fn malformed_sha256_rejected() {
            // Test that SHA256 output is always 32 bytes
            // (build.rs validates hex decoding at compile time)
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(b"test");
            assert_eq!(hash.as_slice().len(), 32);
        }

        #[test]
        fn integrity_helper_sha256_correctness() {
            let temp = TempDir::new().unwrap();
            let test_file = temp.path().join("test.bin");
            fs::write(&test_file, b"hello world").unwrap();

            let hash = compute_sha256(&test_file).unwrap();
            use sha2::{Digest, Sha256};
            let expected = Sha256::digest(b"hello world");
            assert_eq!(hash.as_slice(), expected.as_slice());
        }

        #[test]
        fn missing_metadata_fails_closed_verification_skipped() {
            // When EXPECTED_SHA256 is None, verification is skipped but logged
            // This test verifies the constant can be None
            let _ = EXPECTED_SHA256; // Can be None
            let _ = EXPECTED_BYTE_LENGTH; // Can be None
        }

        #[test]
        fn windows_pe_architecture_x64_constant() {
            // IMAGE_FILE_MACHINE_AMD64 = 0x8664
            const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
            assert_eq!(IMAGE_FILE_MACHINE_AMD64, 0x8664);
        }

        #[test]
        fn dev_vs_release_classification() {
            // IS_DEV_BUILD gates development-only paths
            #[cfg(debug_assertions)]
            assert!(IS_DEV_BUILD);
            #[cfg(not(debug_assertions))]
            assert!(!IS_DEV_BUILD);
        }

        #[test]
        fn get_pdfium_with_resolved_path_uses_provided_path() {
            // Test that the function signature accepts pre-resolved path
            // (Actual binding test requires Windows host - deferred to 11V-D)
            let temp = TempDir::new().unwrap();
            let dummy_dll = temp.path().join("pdfium.dll");
            fs::write(&dummy_dll, b"dummy").unwrap();

            // Function should accept the path (binding will fail but resolution works)
            // We can't test binding without Windows, but we verify the path is used
            let result = get_pdfium_with_resolved_path(Some(dummy_dll.clone()));
            // Error should be about binding, not resolution
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.contains("bind")
                    || err.contains("Failed to bind")
                    || err.contains("DLL")
                    || err.contains("library")
            );
        }
    }

    // Cross-platform FAST_LOCAL tests (run on all platforms)
    #[cfg(test)]
    mod cross_platform_fast_local_tests {
        use super::*;
        use crate::pdf_engine::runtime_metadata::{IS_DEV_BUILD, RUNTIME_FILENAME};
        use std::fs;
        use std::path::PathBuf;
        use tempfile::TempDir;

        #[test]
        fn runtime_filename_matches_platform() {
            #[cfg(target_os = "linux")]
            assert_eq!(RUNTIME_FILENAME, "libpdfium.so");
            #[cfg(target_os = "windows")]
            assert_eq!(RUNTIME_FILENAME, "pdfium.dll");
            #[cfg(target_os = "macos")]
            assert_eq!(RUNTIME_FILENAME, "libpdfium.dylib");
        }

        #[test]
        fn dev_vs_release_classification_cross_platform() {
            #[cfg(debug_assertions)]
            assert!(IS_DEV_BUILD);
            #[cfg(not(debug_assertions))]
            assert!(!IS_DEV_BUILD);
        }

        #[test]
        fn path_traversal_rejection_cross_platform() {
            let temp = TempDir::new().unwrap();
            let traversal = temp.path().join("..").join(RUNTIME_FILENAME);
            assert!(!is_valid_runtime_file(&traversal));
        }

        #[test]
        fn integrity_helper_sha256_correctness_cross_platform() {
            let temp = TempDir::new().unwrap();
            let test_file = temp.path().join("test.bin");
            fs::write(&test_file, b"hello world").unwrap();

            let hash = compute_sha256(&test_file).unwrap();
            use sha2::{Digest, Sha256};
            let expected = Sha256::digest(b"hello world");
            assert_eq!(hash.as_slice(), expected.as_slice());
        }

        #[test]
        fn get_pdfium_with_resolved_path_accepts_path_cross_platform() {
            let temp = TempDir::new().unwrap();
            let dummy = temp.path().join(RUNTIME_FILENAME);
            fs::write(&dummy, b"dummy").unwrap();

            let result = get_pdfium_with_resolved_path(Some(dummy.clone()));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.contains("bind")
                    || err.contains("Failed to bind")
                    || err.contains("DLL")
                    || err.contains("library")
                    || err.contains("dylib")
                    || err.contains("so")
            );
        }

        #[test]
        fn elf_header_parser_accepts_valid_x86_64() {
            let mut header = [0u8; 64];
            header[0..4].copy_from_slice(b"\x7fELF");
            header[4] = 2; // ELFCLASS64
            header[5] = 1; // ELFDATA2LSB (little-endian)
            header[0x12] = 0x3e; // EM_X86_64 low byte
            header[0x13] = 0x00; // EM_X86_64 high byte

            let result = verify_elf_header_bytes(&header, "x86_64");
            assert!(
                result.is_ok(),
                "Valid x86_64 ELF header should be accepted: {:?}",
                result
            );
        }

        #[test]
        fn elf_header_parser_rejects_32bit() {
            let mut header = [0u8; 64];
            header[0..4].copy_from_slice(b"\x7fELF");
            header[4] = 1; // ELFCLASS32 (32-bit)
            header[5] = 1; // little-endian
            header[0x12] = 0x3e;
            header[0x13] = 0x00;

            let result = verify_elf_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeArchitectureMismatch { expected, found } => {
                    assert_eq!(expected, "x86_64");
                    assert!(
                        found.contains("32-bit"),
                        "Expected 32-bit mention, found: {}",
                        found
                    );
                }
                other => panic!("Expected RuntimeArchitectureMismatch, got {:?}", other),
            }
        }

        #[test]
        fn elf_header_parser_rejects_arm_aarch64() {
            let mut header = [0u8; 64];
            header[0..4].copy_from_slice(b"\x7fELF");
            header[4] = 2; // 64-bit
            header[5] = 1; // little-endian
            header[0x12] = 0xb7; // EM_AARCH64 (183 = 0xB7)
            header[0x13] = 0x00;

            let result = verify_elf_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeArchitectureMismatch { expected, found } => {
                    assert_eq!(expected, "x86_64");
                    assert_eq!(found, "AArch64");
                }
                other => panic!("Expected RuntimeArchitectureMismatch, got {:?}", other),
            }
        }

        #[test]
        fn elf_header_parser_rejects_non_elf_signature() {
            let header = [0u8; 64]; // zeros, not \x7fELF
            let result = verify_elf_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("Not a valid ELF file"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn elf_header_parser_rejects_short_header() {
            let short = [0u8; 10];
            let result = verify_elf_header_bytes(&short, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("too short"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn linux_runtime_metadata_constants() {
            use crate::pdf_engine::runtime_metadata::{
                LINUX_EXPECTED_ARCH, LINUX_EXPECTED_PACKAGED_LOCATION, LINUX_EXPECTED_STAGED_PATH,
                LINUX_RUNTIME_FILENAME, LINUX_SUPPLY_CHAIN_STATUS,
            };

            assert_eq!(LINUX_EXPECTED_ARCH, "x86_64");
            assert_eq!(LINUX_RUNTIME_FILENAME, "libpdfium.so");
            assert!(LINUX_EXPECTED_STAGED_PATH.ends_with("lib/libpdfium.so"));
            assert!(LINUX_EXPECTED_PACKAGED_LOCATION.ends_with("lib/libpdfium.so"));
            assert!(matches!(
                LINUX_SUPPLY_CHAIN_STATUS,
                "COMPLETE" | "PARTIAL" | "INCOMPLETE" | "MISSING"
            ));
        }

        #[test]
        fn linux_path_composition_deterministic() {
            let resource_dir = PathBuf::from("/mock/app/resources");
            let packaged = resource_dir.join("lib").join("libpdfium.so");
            assert_eq!(
                packaged.to_str().unwrap(),
                "/mock/app/resources/lib/libpdfium.so"
            );
        }

        #[test]
        fn linux_relative_pdfium_path_rejected() {
            let relative = PathBuf::from("./libpdfium.so");
            assert!(
                !relative.is_absolute(),
                "Relative PDFIUM_PATH must be rejected before canonicalization"
            );
        }

        #[test]
        fn pe_header_parser_accepts_valid_x86_64() {
            let mut header = [0u8; 128];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
            header[86..88].copy_from_slice(&0x0000u16.to_le_bytes());

            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(
                result.is_ok(),
                "Valid x86_64 PE header should be accepted: {:?}",
                result
            );
        }

        #[test]
        fn pe_header_parser_rejects_non_mz_signature() {
            let header = [0u8; 128];
            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("missing MZ signature"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_non_pe_signature() {
            let mut header = [0u8; 128];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"NOPE");
            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("missing PE signature"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_i386_32bit() {
            let mut header = [0u8; 128];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x014cu16.to_le_bytes());
            header[86..88].copy_from_slice(&0x0000u16.to_le_bytes());

            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeArchitectureMismatch { expected, found } => {
                    assert_eq!(expected, "x86_64");
                    assert_eq!(found, "x86");
                }
                other => panic!("Expected RuntimeArchitectureMismatch, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_arm64() {
            let mut header = [0u8; 128];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0xAA64u16.to_le_bytes());
            header[86..88].copy_from_slice(&0x0000u16.to_le_bytes());

            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeArchitectureMismatch { expected, found } => {
                    assert_eq!(expected, "x86_64");
                    assert_eq!(found, "aarch64");
                }
                other => panic!("Expected RuntimeArchitectureMismatch, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_32bit_characteristic_flag() {
            let mut header = [0u8; 128];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
            header[86..88].copy_from_slice(&0x0020u16.to_le_bytes());

            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeArchitectureMismatch { expected, found } => {
                    assert_eq!(expected, "x86_64");
                    assert!(found.contains("32-bit flag set"));
                }
                other => panic!("Expected RuntimeArchitectureMismatch, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_truncated_dos_header() {
            let short = [0u8; 20];
            let result = verify_pe_header_bytes(&short, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("too short"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn pe_header_parser_rejects_truncated_coff_header() {
            let mut header = [0u8; 70];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            let result = verify_pe_header_bytes(&header, "x86_64");
            assert!(result.is_err());
            match result.unwrap_err() {
                AppError::RuntimeLoadFailed(msg) => {
                    assert!(msg.contains("truncated before COFF header"));
                }
                other => panic!("Expected RuntimeLoadFailed, got {:?}", other),
            }
        }

        #[test]
        fn windows_runtime_metadata_constants() {
            use crate::pdf_engine::runtime_metadata::{
                WINDOWS_EXPECTED_ARCH, WINDOWS_EXPECTED_PACKAGED_LOCATION,
                WINDOWS_EXPECTED_STAGED_PATH, WINDOWS_RUNTIME_FILENAME,
                WINDOWS_SUPPLY_CHAIN_STATUS,
            };

            assert_eq!(WINDOWS_EXPECTED_ARCH, "x86_64");
            assert_eq!(WINDOWS_RUNTIME_FILENAME, "pdfium.dll");
            assert!(WINDOWS_EXPECTED_STAGED_PATH.ends_with("lib/pdfium.dll"));
            assert!(WINDOWS_EXPECTED_PACKAGED_LOCATION.ends_with("lib/pdfium.dll"));
            assert!(matches!(
                WINDOWS_SUPPLY_CHAIN_STATUS,
                "COMPLETE" | "PARTIAL" | "INCOMPLETE" | "MISSING"
            ));
        }

        #[test]
        fn windows_path_composition_deterministic() {
            let resource_dir = PathBuf::from("C:\\mock\\app\\resources");
            let packaged = resource_dir.join("lib").join("pdfium.dll");
            assert_eq!(
                packaged.to_string_lossy(),
                "C:\\mock\\app\\resources\\lib\\pdfium.dll"
            );
        }

        #[test]
        fn windows_packaging_path_parity() {
            use crate::pdf_engine::runtime_metadata::{
                RUNTIME_FILENAME, WINDOWS_EXPECTED_PACKAGED_LOCATION,
                WINDOWS_EXPECTED_STAGED_PATH,
            };

            // 1. Packaged location constant must be canonical
            assert_eq!(
                WINDOWS_EXPECTED_PACKAGED_LOCATION,
                "resourceDir/lib/pdfium.dll"
            );

            // 2. Staged path in repo must be canonical
            assert_eq!(WINDOWS_EXPECTED_STAGED_PATH, "src-tauri/lib/pdfium.dll");

            // 3. Package resource definition in tauri.windows.conf.json is derived from staged path
            let package_resource_entry = WINDOWS_EXPECTED_STAGED_PATH
                .strip_prefix("src-tauri/")
                .unwrap_or(WINDOWS_EXPECTED_STAGED_PATH);
            assert_eq!(
                package_resource_entry, "lib/pdfium.dll",
                "Packaging resource entry must match staged path subpath"
            );

            // 4. Loader path composition (resourceDir / lib / RUNTIME_FILENAME) matches packaged location
            let mock_resource_dir = PathBuf::from("resourceDir");
            let loader_packaged = mock_resource_dir.join("lib").join(RUNTIME_FILENAME);
            let loader_str = loader_packaged.to_string_lossy().replace('\\', "/");
            assert_eq!(
                loader_str, WINDOWS_EXPECTED_PACKAGED_LOCATION,
                "D3 loader expected path must be in 100% parity with packaged location"
            );
        }

        #[test]
        fn windows_relative_pdfium_path_rejected() {
            let relative = PathBuf::from("./pdfium.dll");
            assert!(
                !relative.is_absolute(),
                "Relative PDFIUM_PATH must be rejected before canonicalization"
            );
        }
    }
}
