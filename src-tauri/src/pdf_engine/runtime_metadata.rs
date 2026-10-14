include!(concat!(env!("OUT_DIR"), "/runtime_metadata.rs"));

use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub const EXPECTED_SHA256: Option<[u8; 32]> = LINUX_EXPECTED_SHA256;
#[cfg(target_os = "linux")]
pub const EXPECTED_BYTE_LENGTH: Option<u64> = LINUX_EXPECTED_BYTE_LENGTH;
#[cfg(target_os = "linux")]
pub const EXPECTED_VERSION: Option<&str> = LINUX_EXPECTED_VERSION;
#[cfg(target_os = "linux")]
pub const EXPECTED_ARCH: &str = LINUX_EXPECTED_ARCH;
#[cfg(target_os = "linux")]
pub const RUNTIME_FILENAME: &str = LINUX_RUNTIME_FILENAME;
#[cfg(target_os = "linux")]
pub const SUPPLY_CHAIN_STATUS: &str = LINUX_SUPPLY_CHAIN_STATUS;
#[cfg(target_os = "linux")]
pub const EXPECTED_STAGED_PATH: &str = LINUX_EXPECTED_STAGED_PATH;
#[cfg(target_os = "linux")]
pub const EXPECTED_PACKAGED_LOCATION: &str = LINUX_EXPECTED_PACKAGED_LOCATION;

#[cfg(target_os = "windows")]
pub const EXPECTED_SHA256: Option<[u8; 32]> = WINDOWS_EXPECTED_SHA256;
#[cfg(target_os = "windows")]
pub const EXPECTED_BYTE_LENGTH: Option<u64> = WINDOWS_EXPECTED_BYTE_LENGTH;
#[cfg(target_os = "windows")]
pub const EXPECTED_VERSION: Option<&str> = WINDOWS_EXPECTED_VERSION;
#[cfg(target_os = "windows")]
pub const EXPECTED_ARCH: &str = WINDOWS_EXPECTED_ARCH;
#[cfg(target_os = "windows")]
pub const RUNTIME_FILENAME: &str = WINDOWS_RUNTIME_FILENAME;
#[cfg(target_os = "windows")]
pub const SUPPLY_CHAIN_STATUS: &str = WINDOWS_SUPPLY_CHAIN_STATUS;
#[cfg(target_os = "windows")]
pub const EXPECTED_STAGED_PATH: &str = WINDOWS_EXPECTED_STAGED_PATH;
#[cfg(target_os = "windows")]
pub const EXPECTED_PACKAGED_LOCATION: &str = WINDOWS_EXPECTED_PACKAGED_LOCATION;

#[cfg(target_os = "macos")]
pub const EXPECTED_SHA256: Option<[u8; 32]> = None;
#[cfg(target_os = "macos")]
pub const EXPECTED_BYTE_LENGTH: Option<u64> = None;
#[cfg(target_os = "macos")]
pub const EXPECTED_VERSION: Option<&str> = None;
#[cfg(target_os = "macos")]
pub const EXPECTED_ARCH: &str = "arm64";
#[cfg(target_os = "macos")]
pub const RUNTIME_FILENAME: &str = "libpdfium.dylib";
#[cfg(target_os = "macos")]
pub const SUPPLY_CHAIN_STATUS: &str = "INCOMPLETE";
#[cfg(target_os = "macos")]
pub const EXPECTED_STAGED_PATH: &str = "src-tauri/lib/libpdfium.dylib";
#[cfg(target_os = "macos")]
pub const EXPECTED_PACKAGED_LOCATION: &str = "resourceDir/lib/libpdfium.dylib";

pub fn get_expected_staged_path() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(EXPECTED_STAGED_PATH)
}

pub fn get_expected_packaged_path(resource_dir: &std::path::Path) -> PathBuf {
    resource_dir.join("lib").join(RUNTIME_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_defined() {
        let _ = EXPECTED_ARCH;
        let _ = RUNTIME_FILENAME;
        let _ = SUPPLY_CHAIN_STATUS;
        let _ = EXPECTED_STAGED_PATH;
        let _ = EXPECTED_PACKAGED_LOCATION;
        let _ = IS_DEV_BUILD;
    }

    #[test]
    fn arch_is_x86_64_on_linux_and_windows() {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        assert_eq!(EXPECTED_ARCH, "x86_64");
    }

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
    fn supply_chain_status_is_known_value() {
        assert!(matches!(
            SUPPLY_CHAIN_STATUS,
            "COMPLETE" | "PARTIAL" | "INCOMPLETE" | "MISSING"
        ));
    }

    #[test]
    fn staged_path_ends_with_runtime_filename() {
        assert!(EXPECTED_STAGED_PATH.ends_with(RUNTIME_FILENAME));
    }

    #[test]
    fn packaged_location_ends_with_runtime_filename() {
        assert!(EXPECTED_PACKAGED_LOCATION.ends_with(RUNTIME_FILENAME));
    }
}
