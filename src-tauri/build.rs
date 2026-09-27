fn main() {
    tauri_build::build();

    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
        .join("engineering")
        .join("platform-support.json");

    let manifest_content =
        std::fs::read_to_string(&manifest_path).expect("Failed to read platform-support.json");

    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_content).expect("Failed to parse platform-support.json");

    let platforms = manifest
        .get("platforms")
        .and_then(|p| p.as_array())
        .expect("platforms array not found in platform-support.json");

    let linux_runtime = platforms
        .iter()
        .find(|platform| {
            platform.get("targetTriple").and_then(|t| t.as_str())
                == Some("x86_64-unknown-linux-gnu")
        })
        .and_then(|p| p.get("runtime"))
        .expect("Linux x86_64 runtime record not found in platform-support.json");

    let windows_runtime = platforms
        .iter()
        .find(|platform| {
            platform.get("targetTriple").and_then(|t| t.as_str()) == Some("x86_64-pc-windows-msvc")
        })
        .and_then(|p| p.get("runtime"));

    let linux_sha256_str = linux_runtime.get("sha256").and_then(|s| s.as_str());
    let linux_byte_length = linux_runtime.get("byteLength").and_then(|b| b.as_u64());
    let linux_version = linux_runtime
        .get("versionOrRevision")
        .and_then(|v| v.as_str());
    let linux_architecture = linux_runtime
        .get("architecture")
        .and_then(|a| a.as_str())
        .unwrap_or("x86_64");
    let linux_runtime_filename = linux_runtime
        .get("runtimeFileName")
        .and_then(|f| f.as_str())
        .unwrap_or("libpdfium.so");
    let linux_supply_chain_status = linux_runtime
        .get("supplyChainStatus")
        .and_then(|s| s.as_str())
        .unwrap_or("INCOMPLETE");
    let linux_expected_staged_path = linux_runtime
        .get("expectedStagedPath")
        .and_then(|p| p.as_str())
        .unwrap_or("src-tauri/lib/libpdfium.so");
    let linux_expected_packaged_location = linux_runtime
        .get("expectedPackagedLocation")
        .and_then(|p| p.as_str())
        .unwrap_or("resourceDir/lib/libpdfium.so");

    let linux_sha256_bytes = linux_sha256_str.map(|s| {
        let mut bytes = [0u8; 32];
        let decoded = hex::decode(s).expect("Invalid SHA-256 hex");
        bytes.copy_from_slice(&decoded);
        bytes
    });

    let (
        windows_sha256_bytes,
        windows_byte_length,
        windows_version,
        windows_architecture,
        windows_runtime_filename,
        windows_supply_chain_status,
        windows_expected_staged_path,
        windows_expected_packaged_location,
    ) = if let Some(wr) = windows_runtime {
        let sha256_str = wr.get("sha256").and_then(|s| s.as_str());
        let byte_length = wr.get("byteLength").and_then(|b| b.as_u64());
        let version = wr.get("versionOrRevision").and_then(|v| v.as_str());
        let architecture = wr
            .get("architecture")
            .and_then(|a| a.as_str())
            .unwrap_or("x86_64");
        let runtime_filename = wr
            .get("runtimeFileName")
            .and_then(|f| f.as_str())
            .unwrap_or("pdfium.dll");
        let supply_chain_status = wr
            .get("supplyChainStatus")
            .and_then(|s| s.as_str())
            .unwrap_or("INCOMPLETE");
        let expected_staged_path = wr
            .get("expectedStagedPath")
            .and_then(|p| p.as_str())
            .unwrap_or("src-tauri/lib/pdfium.dll");
        let expected_packaged_location = wr
            .get("expectedPackagedLocation")
            .and_then(|p| p.as_str())
            .unwrap_or("resourceDir/lib/pdfium.dll");

        let sha256_bytes = sha256_str.map(|s| {
            let mut bytes = [0u8; 32];
            let decoded = hex::decode(s).expect("Invalid SHA-256 hex");
            bytes.copy_from_slice(&decoded);
            bytes
        });

        (
            sha256_bytes,
            byte_length,
            version,
            architecture,
            runtime_filename,
            supply_chain_status,
            expected_staged_path,
            expected_packaged_location,
        )
    } else {
        (
            None,
            None,
            None,
            "x86_64",
            "pdfium.dll",
            "INCOMPLETE",
            "src-tauri/lib/pdfium.dll",
            "resourceDir/lib/pdfium.dll",
        )
    };

    // Pre-package verification for Windows runtime (release builds only, when targeting Windows)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| {
        std::env::var("TARGET")
            .ok()
            .and_then(|t| t.split('-').nth(2).map(|s| s.to_string()))
            .unwrap_or_else(|| std::env::consts::OS.to_string())
    });

    if !cfg!(debug_assertions) && target_os == "windows" {
        verify_windows_runtime_pre_package(
            &windows_expected_staged_path,
            windows_sha256_bytes,
            windows_byte_length,
            &windows_architecture,
            &windows_supply_chain_status,
        );
    }

    if !cfg!(debug_assertions) && target_os == "linux" {
        verify_linux_runtime_pre_package(
            &linux_expected_staged_path,
            linux_sha256_bytes,
            linux_byte_length,
            &linux_architecture,
            &linux_supply_chain_status,
        );
    }

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest_path = out_dir.join("runtime_metadata.rs");

    let mut content = String::new();
    content.push_str("// Generated by build.rs from platform-support.json\n");
    content.push_str("// DO NOT EDIT MANUALLY\n\n");

    // Linux constants
    content.push_str("/// Expected SHA-256 of the packaged Linux PDFium runtime (libpdfium.so).\n");
    content.push_str("/// None if supply chain metadata is incomplete (current state).\n");
    content.push_str("pub const LINUX_EXPECTED_SHA256: Option<[u8; 32]> = ");
    match linux_sha256_bytes {
        Some(bytes) => {
            content.push_str("Some([");
            for (i, b) in bytes.iter().enumerate() {
                if i > 0 {
                    content.push_str(", ");
                }
                content.push_str(&format!("0x{:02x}", b));
            }
            content.push_str("])\n");
        }
        None => {
            content.push_str("None\n");
        }
    }
    content.push_str(";\n\n");

    content.push_str("/// Expected byte length of the packaged Linux PDFium runtime.\n");
    content.push_str("/// None if supply chain metadata is incomplete.\n");
    content.push_str(&format!(
        "pub const LINUX_EXPECTED_BYTE_LENGTH: Option<u64> = {};\n\n",
        linux_byte_length
            .map(|v| v.to_string())
            .unwrap_or("None".to_string())
    ));

    content.push_str("/// Expected version/revision of the packaged Linux PDFium runtime.\n");
    content.push_str("/// None if supply chain metadata is incomplete.\n");
    content.push_str(&format!(
        "pub const LINUX_EXPECTED_VERSION: Option<&'static str> = {};\n\n",
        linux_version
            .map(|v| format!("\"{}\"", v))
            .unwrap_or("None".to_string())
    ));

    content.push_str("/// Expected architecture of the packaged Linux PDFium runtime.\n");
    content.push_str(&format!(
        "pub const LINUX_EXPECTED_ARCH: &'static str = \"{}\";\n\n",
        linux_architecture
    ));

    content.push_str("/// Expected runtime filename for Linux.\n");
    content.push_str(&format!(
        "pub const LINUX_RUNTIME_FILENAME: &'static str = \"{}\";\n\n",
        linux_runtime_filename
    ));

    content.push_str("/// Supply chain status for Linux runtime.\n");
    content.push_str(&format!(
        "pub const LINUX_SUPPLY_CHAIN_STATUS: &'static str = \"{}\";\n\n",
        linux_supply_chain_status
    ));

    content.push_str("/// Expected staged path in repository (source of truth for bundling).\n");
    content.push_str(&format!(
        "pub const LINUX_EXPECTED_STAGED_PATH: &'static str = \"{}\";\n\n",
        linux_expected_staged_path
    ));

    content.push_str("/// Expected packaged location at runtime (Tauri resourceDir).\n");
    content.push_str(&format!(
        "pub const LINUX_EXPECTED_PACKAGED_LOCATION: &'static str = \"{}\";\n\n",
        linux_expected_packaged_location
    ));

    // Windows constants
    content.push_str("/// Expected SHA-256 of the packaged Windows PDFium runtime (pdfium.dll).\n");
    content.push_str("/// None if supply chain metadata is incomplete (current state).\n");
    content.push_str("pub const WINDOWS_EXPECTED_SHA256: Option<[u8; 32]> = ");
    match windows_sha256_bytes {
        Some(bytes) => {
            content.push_str("Some([");
            for (i, b) in bytes.iter().enumerate() {
                if i > 0 {
                    content.push_str(", ");
                }
                content.push_str(&format!("0x{:02x}", b));
            }
            content.push_str("])\n");
        }
        None => {
            content.push_str("None\n");
        }
    }
    content.push_str(";\n\n");

    content.push_str("/// Expected byte length of the packaged Windows PDFium runtime.\n");
    content.push_str("/// None if supply chain metadata is incomplete.\n");
    content.push_str(&format!(
        "pub const WINDOWS_EXPECTED_BYTE_LENGTH: Option<u64> = {};\n\n",
        windows_byte_length
            .map(|v| v.to_string())
            .unwrap_or("None".to_string())
    ));

    content.push_str("/// Expected version/revision of the packaged Windows PDFium runtime.\n");
    content.push_str("/// None if supply chain metadata is incomplete.\n");
    content.push_str(&format!(
        "pub const WINDOWS_EXPECTED_VERSION: Option<&'static str> = {};\n\n",
        windows_version
            .map(|v| format!("\"{}\"", v))
            .unwrap_or("None".to_string())
    ));

    content.push_str("/// Expected architecture of the packaged Windows PDFium runtime.\n");
    content.push_str(&format!(
        "pub const WINDOWS_EXPECTED_ARCH: &'static str = \"{}\";\n\n",
        windows_architecture
    ));

    content.push_str("/// Expected runtime filename for Windows.\n");
    content.push_str(&format!(
        "pub const WINDOWS_RUNTIME_FILENAME: &'static str = \"{}\";\n\n",
        windows_runtime_filename
    ));

    content.push_str("/// Supply chain status for Windows runtime.\n");
    content.push_str(&format!(
        "pub const WINDOWS_SUPPLY_CHAIN_STATUS: &'static str = \"{}\";\n\n",
        windows_supply_chain_status
    ));

    content.push_str("/// Expected staged path in repository (source of truth for bundling).\n");
    content.push_str(&format!(
        "pub const WINDOWS_EXPECTED_STAGED_PATH: &'static str = \"{}\";\n\n",
        windows_expected_staged_path
    ));

    content.push_str("/// Expected packaged location at runtime (Tauri resourceDir).\n");
    content.push_str(&format!(
        "pub const WINDOWS_EXPECTED_PACKAGED_LOCATION: &'static str = \"{}\";\n\n",
        windows_expected_packaged_location
    ));

    content.push_str("/// Whether the current build is a development build (debug_assertions).\n");
    content.push_str(&format!(
        "pub const IS_DEV_BUILD: bool = {};\n",
        cfg!(debug_assertions)
    ));

    std::fs::write(&dest_path, content).expect("Failed to write runtime_metadata.rs");

    println!("cargo:rerun-if-changed=../docs/engineering/platform-support.json");
    println!("cargo:rerun-if-changed=../src-tauri/lib/pdfium.dll");
    println!("cargo:rerun-if-changed=../src-tauri/lib/libpdfium.so");
}

fn verify_windows_runtime_pre_package(
    staged_path: &str,
    expected_sha256: Option<[u8; 32]>,
    expected_byte_length: Option<u64>,
    expected_architecture: &str,
    supply_chain_status: &str,
) {
    use std::fs;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let staged_dll_path = manifest_dir.join("..").join(staged_path);

    // 1. Release eligibility gate: supply chain status must be COMPLETE for release builds
    if supply_chain_status != "COMPLETE" {
        panic!(
            "RELEASE_ELIGIBILITY_GATE_FAILED: Windows runtime supply chain status is '{}', expected 'COMPLETE' for release build. \
            Provision a verified Windows PDFium DLL and update platform-support.json with sha256, byteLength, versionOrRevision, and supplyChainStatus: COMPLETE.",
            supply_chain_status
        );
    }

    // 2. Verify staged DLL exists
    // The DLL must be provisioned to this location before build.rs runs.
    if !staged_dll_path.exists() {
        panic!(
            "MISSING_RUNTIME_ARTIFACT: Windows PDFium DLL not found at staged path: {}. \
            Provision the verified x86_64 pdfium.dll to this location before building.",
            staged_dll_path.display()
        );
    }

    // 3. Verify filename matches expected
    let filename = staged_dll_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if filename != "pdfium.dll" {
        panic!(
            "INVALID_RUNTIME_FILENAME: Staged DLL has unexpected filename '{}', expected 'pdfium.dll'",
            filename
        );
    }

    // 4. Verify byte length if expected
    if let Some(expected_len) = expected_byte_length {
        let metadata = fs::metadata(&staged_dll_path).expect("Failed to read staged DLL metadata");
        if metadata.len() != expected_len {
            panic!(
                "INTEGRITY_MISMATCH: Byte length mismatch for staged Windows PDFium DLL. \
                Expected {} bytes, found {} bytes.",
                expected_len,
                metadata.len()
            );
        }
    }

    // 5. Verify SHA-256 if expected
    if let Some(expected_sha) = expected_sha256 {
        let actual_sha =
            compute_sha256(&staged_dll_path).expect("Failed to compute SHA-256 of staged DLL");
        if actual_sha != expected_sha {
            panic!(
                "INTEGRITY_MISMATCH: SHA-256 mismatch for staged Windows PDFium DLL. \
                Expected {:02x?}, found {:02x?}.",
                expected_sha, actual_sha
            );
        }
    }

    // 6. Verify PE architecture (x86_64 / IMAGE_FILE_MACHINE_AMD64)
    verify_windows_dll_architecture(&staged_dll_path, expected_architecture);

    println!(
        "cargo:warning=Windows PDFium DLL pre-package verification PASSED: {}",
        staged_dll_path.display()
    );
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

fn verify_windows_dll_architecture(path: &std::path::Path, expected_arch: &str) {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut file =
        File::open(path).expect("Failed to open staged DLL for architecture verification");

    // Read DOS header to find PE header offset
    let mut dos_header = [0u8; 64];
    file.read_exact(&mut dos_header)
        .expect("Failed to read DOS header from staged DLL");

    if &dos_header[0..2] != b"MZ" {
        panic!("INTEGRITY_MISMATCH: Staged DLL is not a valid PE file (missing MZ signature)");
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
        .expect("Failed to seek to PE header in staged DLL");

    // Read PE signature (4 bytes: "PE\0\0")
    let mut pe_sig = [0u8; 4];
    file.read_exact(&mut pe_sig)
        .expect("Failed to read PE signature from staged DLL");

    if &pe_sig != b"PE\0\0" {
        panic!("INTEGRITY_MISMATCH: Staged DLL is not a valid PE file (missing PE signature)");
    }

    // Read COFF File Header (20 bytes)
    let mut coff_header = [0u8; 20];
    file.read_exact(&mut coff_header)
        .expect("Failed to read COFF header from staged DLL");

    // Machine field is first 2 bytes (little-endian)
    let machine = u16::from_le_bytes([coff_header[0], coff_header[1]]);

    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

    let found_arch = match machine {
        IMAGE_FILE_MACHINE_AMD64 => "x86_64",
        IMAGE_FILE_MACHINE_I386 => "x86",
        IMAGE_FILE_MACHINE_ARM64 => "aarch64",
        _ => "unknown",
    };

    if expected_arch == "x86_64" && machine != IMAGE_FILE_MACHINE_AMD64 {
        panic!(
            "ARCHITECTURE_MISMATCH: Staged Windows PDFium DLL architecture is '{}', expected 'x86_64' (IMAGE_FILE_MACHINE_AMD64). \
            Cross-architecture DLL contamination detected.",
            found_arch
        );
    }

    // Verify it's 64-bit (not 32-bit) by checking Characteristics
    let characteristics = u16::from_le_bytes([coff_header[18], coff_header[19]]);
    const IMAGE_FILE_32BIT_MACHINE: u16 = 0x0020;
    if characteristics & IMAGE_FILE_32BIT_MACHINE != 0 {
        panic!(
            "ARCHITECTURE_MISMATCH: Staged Windows PDFium DLL has 32-bit flag set (IMAGE_FILE_32BIT_MACHINE). \
            Expected 64-bit x86_64 DLL."
        );
    }
}

fn verify_linux_runtime_pre_package(
    staged_path: &str,
    expected_sha256: Option<[u8; 32]>,
    expected_byte_length: Option<u64>,
    expected_architecture: &str,
    supply_chain_status: &str,
) {
    use std::fs;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let staged_so_path = manifest_dir.join("..").join(staged_path);

    // 1. Release eligibility gate: supply chain status must be COMPLETE for release builds
    if supply_chain_status != "COMPLETE" {
        panic!(
            "RELEASE_ELIGIBILITY_GATE_FAILED: Linux runtime supply chain status is '{}', expected 'COMPLETE' for release build. \
            Provision a verified Linux PDFium shared library and update platform-support.json with sha256, byteLength, versionOrRevision, and supplyChainStatus: COMPLETE.",
            supply_chain_status
        );
    }

    // 2. Staging clean step: remove any existing staged shared library if needed
    // In practice, the shared library should be placed by provisioning step BEFORE build.rs runs.
    if !staged_so_path.exists() {
        panic!(
            "MISSING_RUNTIME_ARTIFACT: Linux PDFium shared library not found at staged path: {}. \
            Provision the verified x86_64 libpdfium.so to this location before building.",
            staged_so_path.display()
        );
    }

    // 3. Verify filename matches expected
    let filename = staged_so_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if filename != "libpdfium.so" {
        panic!(
            "INVALID_RUNTIME_FILENAME: Staged shared library has unexpected filename '{}', expected 'libpdfium.so'",
            filename
        );
    }

    // 4. Verify byte length if expected
    if let Some(expected_len) = expected_byte_length {
        let metadata =
            fs::metadata(&staged_so_path).expect("Failed to read staged shared library metadata");
        if metadata.len() != expected_len {
            panic!(
                "INTEGRITY_MISMATCH: Byte length mismatch for staged Linux PDFium shared library. \
                Expected {} bytes, found {} bytes.",
                expected_len,
                metadata.len()
            );
        }
    }

    // 5. Verify SHA-256 if expected
    if let Some(expected_sha) = expected_sha256 {
        let actual_sha = compute_sha256(&staged_so_path)
            .expect("Failed to compute SHA-256 of staged shared library");
        if actual_sha != expected_sha {
            panic!(
                "INTEGRITY_MISMATCH: SHA-256 mismatch for staged Linux PDFium shared library. \
                Expected {:02x?}, found {:02x?}.",
                expected_sha, actual_sha
            );
        }
    }

    // 6. Verify ELF architecture (x86_64 / EM_X86_64)
    verify_linux_so_architecture(&staged_so_path, expected_architecture);

    println!(
        "cargo:warning=Linux PDFium shared library pre-package verification PASSED: {}",
        staged_so_path.display()
    );
}

fn verify_linux_so_architecture(path: &std::path::Path, expected_arch: &str) {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path)
        .expect("Failed to open staged shared library for architecture verification");

    let mut elf_header = [0u8; 64];
    file.read_exact(&mut elf_header)
        .expect("Failed to read ELF header from staged shared library");

    if &elf_header[0..4] != b"\x7fELF" {
        panic!("INTEGRITY_MISMATCH: Staged shared library is not a valid ELF file (missing \\x7fELF signature)");
    }

    let ei_class = elf_header[4];
    let ei_data = elf_header[5];

    if ei_class != 2 {
        panic!("ARCHITECTURE_MISMATCH: Staged Linux shared library is 32-bit (ELFCLASS32), expected 64-bit (ELFCLASS64).");
    }

    let is_little_endian = ei_data == 1;
    let machine_bytes = [elf_header[0x12], elf_header[0x13]];
    let e_machine = if is_little_endian {
        u16::from_le_bytes(machine_bytes)
    } else {
        u16::from_be_bytes(machine_bytes)
    };

    const EM_X86_64: u16 = 0x3e;
    if expected_arch == "x86_64" && e_machine != EM_X86_64 {
        let found_arch = match e_machine {
            0x03 => "i386",
            0x28 => "ARM",
            0xb7 => "AArch64",
            0xf3 => "RISC-V",
            _ => "unknown",
        };
        panic!(
            "ARCHITECTURE_MISMATCH: Staged Linux PDFium shared library architecture is '{}', expected 'x86_64' (EM_X86_64). \
            Cross-architecture library contamination detected.",
            found_arch
        );
    }
}
