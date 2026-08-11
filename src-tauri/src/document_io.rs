//! Atomic single-target PDF output orchestration.
//!
//! Temporary files live beside the target. RAII removes them on ordinary success,
//! error returns, and unwinding builds. The release profile uses `panic = "abort"`,
//! so process aborts and machine crashes can leave an orphan temp or recovery backup.
//! This module deliberately does not delete orphan-looking files by name or age;
//! safe scavenging requires a future ownership journal/lease design.
//!
//! Serialization is limited to operations that participate in this module's shared
//! path-lock registry. Writers that still save directly do not receive a process-wide
//! race guarantee against these transactions.

use crate::pdf_engine::with_pdfium;
use parking_lot::Mutex;
use pdfium_render::prelude::{PdfDocument, PdfSecurityHandlerRevision, Pdfium};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};
#[cfg(not(test))]
use std::time::Duration;
use std::time::SystemTime;
use tempfile::TempPath;

const TEMP_PREFIX: &str = ".antigravity-pdf-";
const TEMP_SUFFIX: &str = ".pdf";
const PATH_LOCK_RETRIES: usize = 3;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentIoStage {
    Preflight,
    CreateTemp,
    WritePdf,
    ApplyMetadata,
    SyncTemp,
    ValidateOpen,
    ValidateInvariant,
    RevalidatePaths,
    Replace,
    PostCommitVerify,
    Recover,
    SyncDirectory,
    Cleanup,
}

impl fmt::Display for DocumentIoStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Preflight => "preflight",
            Self::CreateTemp => "create_temp",
            Self::WritePdf => "write_pdf",
            Self::ApplyMetadata => "apply_metadata",
            Self::SyncTemp => "sync_temp",
            Self::ValidateOpen => "validate_open",
            Self::ValidateInvariant => "validate_invariant",
            Self::RevalidatePaths => "revalidate_paths",
            Self::Replace => "replace",
            Self::PostCommitVerify => "post_commit_verify",
            Self::Recover => "recover",
            Self::SyncDirectory => "sync_directory",
            Self::Cleanup => "cleanup",
        };
        formatter.write_str(value)
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitState {
    Prepared,
    OriginalPreserved,
    OriginalRecovered,
    NewCommitted,
    NewCommittedDurabilityUnconfirmed,
    RecoveryRequired,
    Indeterminate,
}

impl fmt::Display for CommitState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Prepared => "prepared",
            Self::OriginalPreserved => "original_preserved",
            Self::OriginalRecovered => "original_recovered",
            Self::NewCommitted => "new_committed",
            Self::NewCommittedDurabilityUnconfirmed => "new_committed_durability_unconfirmed",
            Self::RecoveryRequired => "recovery_required",
            Self::Indeterminate => "indeterminate",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug)]
pub(crate) struct DocumentIoFailure {
    pub stage: DocumentIoStage,
    pub detail: String,
}

impl DocumentIoFailure {
    pub(crate) fn new(stage: DocumentIoStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }

    pub(crate) fn preflight(detail: impl Into<String>) -> Self {
        Self::new(DocumentIoStage::Preflight, detail)
    }

    pub(crate) fn write(detail: impl Into<String>) -> Self {
        Self::new(DocumentIoStage::WritePdf, detail)
    }

    pub(crate) fn invariant(detail: impl Into<String>) -> Self {
        Self::new(DocumentIoStage::ValidateInvariant, detail)
    }
}

impl From<String> for DocumentIoFailure {
    fn from(detail: String) -> Self {
        Self::write(detail)
    }
}

#[derive(Debug)]
pub(crate) struct DocumentIoError {
    pub operation: String,
    pub stage: DocumentIoStage,
    pub state: CommitState,
    pub detail: String,
    pub cleanup_error: Option<String>,
    pub recovery_path: Option<Box<PathBuf>>,
    pub replacement_digest: Option<String>,
}

impl DocumentIoError {
    pub(crate) fn new(
        operation: &str,
        stage: DocumentIoStage,
        state: CommitState,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation: operation.to_string(),
            stage,
            state,
            detail: detail.into(),
            cleanup_error: None,
            recovery_path: None,
            replacement_digest: None,
        }
    }

    fn with_cleanup_error(mut self, error: std::io::Error) -> Self {
        self.cleanup_error = Some(error.to_string());
        self
    }

    pub(crate) fn with_recovery_path(mut self, path: PathBuf) -> Self {
        self.recovery_path = Some(Box::new(path));
        self
    }

    pub(crate) fn with_replacement_digest(mut self, digest: String) -> Self {
        self.replacement_digest = Some(digest);
        self
    }
}

impl fmt::Display for DocumentIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DOCUMENT_IO_ERROR operation={} stage={} state={}: {}",
            self.operation, self.stage, self.state, self.detail
        )?;
        if let Some(path) = &self.recovery_path {
            write!(formatter, "; recovery_path={}", path.display())?;
        }
        if let Some(error) = &self.cleanup_error {
            write!(formatter, "; cleanup_failed={error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DocumentIoError {}

#[derive(Debug)]
pub(crate) struct AtomicWriteReport {
    pub state: CommitState,
    pub committed_digest: String,
}

#[derive(Debug)]
struct AtomicReplaceOutcome {
    report: AtomicWriteReport,
    recovery_backup: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectoryDurability {
    Confirmed,
    Unsupported,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum FileIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows { volume: u32, index: u64 },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum PathLockKey {
    Namespace(PathBuf),
    Identity(FileIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SnapshotKind {
    Missing,
    RegularFile,
}

#[derive(Clone, Debug)]
struct FileSnapshot {
    kind: SnapshotKind,
    identity: Option<FileIdentity>,
    len: u64,
    modified: Option<SystemTime>,
    links: Option<u64>,
}

impl FileSnapshot {
    fn missing() -> Self {
        Self {
            kind: SnapshotKind::Missing,
            identity: None,
            len: 0,
            modified: None,
            links: None,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.identity == other.identity
            && self.len == other.len
            && self.modified == other.modified
    }
}

#[derive(Clone, Debug)]
struct ResolvedPath {
    path: PathBuf,
    namespace: PathBuf,
    snapshot: FileSnapshot,
}

#[derive(Clone, Debug)]
struct ResolvedOperationPaths {
    sources: Vec<ResolvedPath>,
    target: ResolvedPath,
    lock_keys: Vec<PathLockKey>,
}

static PATH_LOCKS: OnceLock<Mutex<HashMap<PathLockKey, Weak<Mutex<()>>>>> = OnceLock::new();

fn path_lock_registry() -> &'static Mutex<HashMap<PathLockKey, Weak<Mutex<()>>>> {
    PATH_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn locks_for(keys: &[PathLockKey]) -> Vec<Arc<Mutex<()>>> {
    let mut registry = path_lock_registry().lock();
    registry.retain(|_, lock| lock.strong_count() > 0);
    keys.iter()
        .map(|key| {
            if let Some(lock) = registry.get(key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                registry.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        })
        .collect()
}

#[cfg(unix)]
fn metadata_identity(metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(FileIdentity::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn metadata_link_count(metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.nlink())
}

#[cfg(not(unix))]
fn metadata_identity(_metadata: &Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(not(unix))]
fn metadata_link_count(_metadata: &Metadata) -> Option<u64> {
    None
}

#[cfg(windows)]
fn windows_identity_and_link_count(path: &Path) -> Result<(FileIdentity, u64), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = File::open(path)
        .map_err(|error| format!("Windows file identity handle açılamadı: {error}"))?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if succeeded == 0 {
        return Err(format!(
            "Windows file identity okunamadı: {}",
            std::io::Error::last_os_error()
        ));
    }

    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((
        FileIdentity::Windows {
            volume: information.dwVolumeSerialNumber,
            index,
        },
        u64::from(information.nNumberOfLinks),
    ))
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn namespace_key(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(windows))]
fn namespace_key(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn snapshot_regular_file(path: &Path, reject_links: bool) -> Result<FileSnapshot, String> {
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSnapshot::missing())
        }
        Err(error) => {
            return Err(format!(
                "{} metadata bilgisi okunamadı: {}",
                path.display(),
                error
            ))
        }
    };

    if (link_metadata.file_type().is_symlink() || is_reparse_point(&link_metadata)) && reject_links
    {
        return Err(format!(
            "Sembolik link/reparse-point çıktı hedefleri desteklenmiyor: {}",
            path.display()
        ));
    }

    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "{} hedefi çözümlenemedi; dangling link olabilir: {}",
            path.display(),
            error
        )
    })?;
    if !metadata.is_file() {
        return Err(format!("Yol regular file değil: {}", path.display()));
    }

    #[cfg(windows)]
    let (identity, links) = windows_identity_and_link_count(path)?;
    #[cfg(not(windows))]
    let (identity, links) = (
        metadata_identity(&metadata)
            .ok_or_else(|| format!("Dosya kimliği belirlenemedi: {}", path.display()))?,
        metadata_link_count(&metadata).unwrap_or(1),
    );

    Ok(FileSnapshot {
        kind: SnapshotKind::RegularFile,
        identity: Some(identity),
        len: metadata.len(),
        modified: metadata.modified().ok(),
        links: Some(links),
    })
}

fn normalize_missing_path(path: &Path) -> Result<PathBuf, String> {
    let filename = path
        .file_name()
        .ok_or_else(|| format!("Hedef dosya adı geçersiz: {}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|error| {
        format!(
            "Hedef parent dizini bulunamadı veya erişilemiyor ({}): {}",
            parent.display(),
            error
        )
    })?;
    if !parent.is_dir() {
        return Err(format!(
            "Hedef parent yolu bir dizin değil: {}",
            parent.display()
        ));
    }
    Ok(parent.join(filename))
}

fn resolve_source(path: &Path) -> Result<ResolvedPath, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Kaynak yol çözümlenemedi ({}): {}", path.display(), error))?;
    let snapshot = snapshot_regular_file(&canonical, false)?;
    if snapshot.kind != SnapshotKind::RegularFile {
        return Err(format!("Kaynak dosya bulunamadı: {}", path.display()));
    }
    Ok(ResolvedPath {
        namespace: namespace_key(&canonical),
        path: canonical,
        snapshot,
    })
}

fn resolve_target(path: &Path) -> Result<ResolvedPath, String> {
    let initial_snapshot = snapshot_regular_file(path, true)?;
    let normalized = if initial_snapshot.kind == SnapshotKind::RegularFile {
        path.canonicalize()
            .map_err(|error| format!("Hedef yol çözümlenemedi ({}): {}", path.display(), error))?
    } else {
        normalize_missing_path(path)?
    };
    let snapshot = snapshot_regular_file(&normalized, true)?;
    if snapshot.links.unwrap_or(1) > 1 {
        return Err(format!(
            "Birden fazla hard-link'i olan çıktı hedefleri desteklenmiyor: {}",
            normalized.display()
        ));
    }
    Ok(ResolvedPath {
        namespace: namespace_key(&normalized),
        path: normalized,
        snapshot,
    })
}

/// Compares two persistence targets using the same canonical namespace and file
/// identity rules used by the atomic path-lock authority.
pub(crate) fn same_persistence_target(left: &Path, right: &Path) -> Result<bool, String> {
    let left = resolve_target(left)?;
    let right = resolve_target(right)?;
    Ok(left.namespace == right.namespace
        || (left.snapshot.identity.is_some() && left.snapshot.identity == right.snapshot.identity))
}

fn resolve_operation_paths(
    sources: &[PathBuf],
    target: &Path,
) -> Result<ResolvedOperationPaths, String> {
    let sources = sources
        .iter()
        .map(|source| resolve_source(source))
        .collect::<Result<Vec<_>, _>>()?;
    let target = resolve_target(target)?;

    let mut keys = BTreeSet::new();
    for path in sources.iter().chain(std::iter::once(&target)) {
        keys.insert(PathLockKey::Namespace(path.namespace.clone()));
        if let Some(identity) = &path.snapshot.identity {
            keys.insert(PathLockKey::Identity(identity.clone()));
        }
    }

    Ok(ResolvedOperationPaths {
        sources,
        target,
        lock_keys: keys.into_iter().collect(),
    })
}

fn operation_snapshots_match(
    expected: &ResolvedOperationPaths,
    current: &ResolvedOperationPaths,
) -> bool {
    expected.sources.len() == current.sources.len()
        && expected
            .sources
            .iter()
            .zip(&current.sources)
            .all(|(left, right)| {
                left.namespace == right.namespace && left.snapshot.matches(&right.snapshot)
            })
        && expected.target.namespace == current.target.namespace
        && expected.target.snapshot.matches(&current.target.snapshot)
}

struct TempOutput {
    path: Option<TempPath>,
}

impl TempOutput {
    fn create(parent: &Path) -> std::io::Result<Self> {
        let file = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .suffix(TEMP_SUFFIX)
            .tempfile_in(parent)?;
        Ok(Self {
            path: Some(file.into_temp_path()),
        })
    }

    fn path(&self) -> &Path {
        self.path
            .as_ref()
            .expect("temp output path should exist")
            .as_ref()
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        if let Some(path) = self.path.take() {
            path.close()
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    fn preserve(&mut self) -> Result<PathBuf, std::io::Error> {
        match self.path.take() {
            Some(path) => path.keep().map_err(|error| error.error),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "temp output path already consumed",
            )),
        }
    }
}

fn cleanup_failure(temp: &mut TempOutput, mut error: DocumentIoError) -> DocumentIoError {
    if let Err(cleanup_error) = temp.cleanup() {
        error = error.with_cleanup_error(cleanup_error);
    }
    error
}

fn apply_target_permissions(
    target_snapshot: &FileSnapshot,
    target: &Path,
    temp: &Path,
) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = if target_snapshot.kind == SnapshotKind::RegularFile {
            fs::metadata(target)?.permissions()
        } else {
            fs::Permissions::from_mode(0o600)
        };
        fs::set_permissions(temp, permissions)?;
    }
    let _ = (target_snapshot, target, temp);
    Ok(())
}

fn sync_temp(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<DirectoryDurability, std::io::Error> {
    File::open(parent)?.sync_all()?;
    Ok(DirectoryDurability::Confirmed)
}

#[cfg(windows)]
fn sync_parent_directory(_parent: &Path) -> Result<DirectoryDurability, std::io::Error> {
    // ReplaceFileW's REPLACEFILE_WRITE_THROUGH flag is documented as unsupported.
    // FlushFileBuffers documents file/volume handles, not a user-mode directory
    // rename durability guarantee. Do not promote this to confirmed durability.
    Ok(DirectoryDurability::Unsupported)
}

#[cfg(not(any(unix, windows)))]
fn sync_parent_directory(_parent: &Path) -> Result<DirectoryDurability, std::io::Error> {
    Ok(DirectoryDurability::Unsupported)
}

#[allow(dead_code)]
pub(crate) fn validate_windows_dacl_security(_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

        let wide_path: Vec<u16> = _path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut p_dacl: *mut windows_sys::Win32::Security::ACL = std::ptr::null_mut();

        let res = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut p_dacl,
                std::ptr::null_mut(),
                &mut p_sec_desc,
            )
        };

        if res != 0 || p_sec_desc.is_null() {
            return Err("DACL security info query failed".to_string());
        }

        struct SecDescGuard(PSECURITY_DESCRIPTOR);
        impl Drop for SecDescGuard {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { windows_sys::Win32::Foundation::LocalFree(self.0 as _) };
                }
            }
        }
        let _guard = SecDescGuard(p_sec_desc);

        if p_dacl.is_null() {
            return Err(
                "UNSAFE_WINDOWS_DACL: NULL DACL provides unprotected full access".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_general_pdf(path: &Path) -> Result<(), DocumentIoFailure> {
    let metadata = fs::metadata(path).map_err(|error| {
        DocumentIoFailure::new(
            DocumentIoStage::ValidateOpen,
            format!("Temp PDF metadata okunamadı: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(DocumentIoFailure::new(
            DocumentIoStage::ValidateOpen,
            "Temp PDF regular file değil",
        ));
    }
    if metadata.len() == 0 {
        return Err(DocumentIoFailure::new(
            DocumentIoStage::ValidateOpen,
            "Temp PDF boş",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_unprotected(document: &PdfDocument<'_>) -> Result<(), DocumentIoFailure> {
    match document.permissions().security_handler_revision() {
        Ok(PdfSecurityHandlerRevision::Unprotected) => Ok(()),
        Ok(revision) => Err(DocumentIoFailure::preflight(format!(
            "Şifreli PDF mutasyonları henüz desteklenmiyor (security handler: {revision:?})"
        ))),
        Err(error) => Err(DocumentIoFailure::preflight(format!(
            "PDF security-handler durumu güvenli biçimde belirlenemedi: {error}"
        ))),
    }
}

fn atomic_write_with<T, W, V, P>(
    operation: &str,
    source_paths: &[PathBuf],
    requested_target: &Path,
    write: W,
    validate: V,
    precondition: P,
) -> Result<AtomicWriteReport, DocumentIoError>
where
    W: FnOnce(&Path) -> Result<T, DocumentIoFailure>,
    V: FnOnce(&Path, &T) -> Result<(), DocumentIoFailure>,
    P: Fn(&Path) -> Result<(), DocumentIoFailure>,
{
    let mut write = Some(write);
    let mut validate = Some(validate);

    for attempt in 0..PATH_LOCK_RETRIES {
        let preliminary =
            resolve_operation_paths(source_paths, requested_target).map_err(|detail| {
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::Preflight,
                    CommitState::Prepared,
                    detail,
                )
            })?;
        let locks = locks_for(&preliminary.lock_keys);
        let _guards = locks.iter().map(|lock| lock.lock()).collect::<Vec<_>>();
        let authoritative =
            resolve_operation_paths(source_paths, requested_target).map_err(|detail| {
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::Preflight,
                    CommitState::Prepared,
                    detail,
                )
            })?;

        if preliminary.lock_keys != authoritative.lock_keys {
            if attempt + 1 == PATH_LOCK_RETRIES {
                return Err(DocumentIoError::new(
                    operation,
                    DocumentIoStage::Preflight,
                    CommitState::Prepared,
                    "PRECONDITION_CONCURRENT_PATH_CHANGE: yol kimliği kararlı hale gelmedi",
                ));
            }
            continue;
        }

        let target = &authoritative.target.path;
        precondition(target).map_err(|failure| {
            DocumentIoError::new(
                operation,
                failure.stage,
                CommitState::OriginalPreserved,
                failure.detail,
            )
        })?;
        let parent = target.parent().ok_or_else(|| {
            DocumentIoError::new(
                operation,
                DocumentIoStage::Preflight,
                CommitState::Prepared,
                "Hedef parent dizini belirlenemedi",
            )
        })?;
        let mut temp = TempOutput::create(parent).map_err(|error| {
            DocumentIoError::new(
                operation,
                DocumentIoStage::CreateTemp,
                CommitState::Prepared,
                format!("Hedef yakınında temp oluşturulamadı: {error}"),
            )
        })?;

        let expected = match write.take().expect("write closure consumed once")(temp.path()) {
            Ok(expected) => expected,
            Err(failure) => {
                return Err(cleanup_failure(
                    &mut temp,
                    DocumentIoError::new(
                        operation,
                        failure.stage,
                        CommitState::OriginalPreserved,
                        failure.detail,
                    ),
                ))
            }
        };

        if let Err(error) =
            apply_target_permissions(&authoritative.target.snapshot, target, temp.path())
        {
            return Err(cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::ApplyMetadata,
                    CommitState::OriginalPreserved,
                    error.to_string(),
                ),
            ));
        }

        if let Err(error) = sync_temp(temp.path()) {
            return Err(cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::SyncTemp,
                    CommitState::OriginalPreserved,
                    error.to_string(),
                ),
            ));
        }

        if let Err(failure) =
            validate.take().expect("validate closure consumed once")(temp.path(), &expected)
        {
            return Err(cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    failure.stage,
                    CommitState::OriginalPreserved,
                    failure.detail,
                ),
            ));
        }

        let replacement_digest = sha256_file(temp.path()).map_err(|error| {
            cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::ValidateInvariant,
                    CommitState::OriginalPreserved,
                    format!("Validated replacement SHA-256 hesaplanamadı: {error}"),
                ),
            )
        })?;

        let before_commit =
            resolve_operation_paths(source_paths, requested_target).map_err(|detail| {
                cleanup_failure(
                    &mut temp,
                    DocumentIoError::new(
                        operation,
                        DocumentIoStage::RevalidatePaths,
                        CommitState::OriginalPreserved,
                        detail,
                    ),
                )
            })?;
        if !operation_snapshots_match(&authoritative, &before_commit) {
            return Err(cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::RevalidatePaths,
                    CommitState::OriginalPreserved,
                    "PRECONDITION_TARGET_CHANGED: source veya target işlem sırasında değişti",
                ),
            ));
        }
        precondition(target).map_err(|failure| {
            cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    failure.stage,
                    CommitState::OriginalPreserved,
                    failure.detail,
                ),
            )
        })?;

        let replacement_snapshot = snapshot_regular_file(temp.path(), false).map_err(|detail| {
            cleanup_failure(
                &mut temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::Replace,
                    CommitState::OriginalPreserved,
                    format!("Validated replacement kimliği okunamadı: {detail}"),
                ),
            )
        })?;
        let mut outcome = atomic_replace(
            operation,
            &mut temp,
            target,
            &authoritative.target.snapshot,
            &replacement_snapshot,
        )
        .map_err(|error| error.with_replacement_digest(replacement_digest.clone()))?;
        outcome.report.committed_digest = replacement_digest.clone();

        let committed_target = snapshot_regular_file(target, true).map_err(|detail| {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::PostCommitVerify,
                CommitState::Indeterminate,
                format!("Commit sonrası target kimliği okunamadı: {detail}"),
            );
            if let Some(path) = outcome.recovery_backup.take() {
                error = error.with_recovery_path(path);
            }
            error.with_replacement_digest(replacement_digest.clone())
        })?;
        if committed_target.identity.is_none()
            || committed_target.identity != replacement_snapshot.identity
        {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::PostCommitVerify,
                CommitState::Indeterminate,
                "Commit sonrası target validated replacement ile eşleşmiyor",
            );
            if let Some(path) = outcome.recovery_backup.take() {
                error = error.with_recovery_path(path);
            }
            return Err(error.with_replacement_digest(replacement_digest));
        }
        let committed_digest = sha256_file(target).map_err(|error| {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::PostCommitVerify,
                CommitState::Indeterminate,
                format!("Commit sonrası target SHA-256 okunamadı: {error}"),
            );
            if let Some(path) = outcome.recovery_backup.take() {
                error = error.with_recovery_path(path);
            }
            error.with_replacement_digest(replacement_digest.clone())
        })?;
        if committed_digest != replacement_digest {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::PostCommitVerify,
                CommitState::Indeterminate,
                "Commit sonrası target digest'i validated replacement ile eşleşmiyor",
            );
            if let Some(path) = outcome.recovery_backup.take() {
                error = error.with_recovery_path(path);
            }
            return Err(error.with_replacement_digest(replacement_digest));
        }

        match sync_parent_directory(parent) {
            Ok(DirectoryDurability::Confirmed) => {
                outcome.report.state = CommitState::NewCommitted;
                if let Some(backup) = outcome.recovery_backup.take() {
                    if let Err(error) = fs::remove_file(&backup) {
                        tracing::warn!(
                            code = "DOCUMENT_IO_COMMITTED_BACKUP_RETAINED",
                            operation,
                            target = %target.display(),
                            recovery_path = %backup.display(),
                            error = %error,
                            "Commit doğrulandı ancak recovery backup silinemedi"
                        );
                    }
                }
            }
            Ok(DirectoryDurability::Unsupported) => {
                outcome.report.state = CommitState::NewCommittedDurabilityUnconfirmed;
                tracing::warn!(
                    code = "DOCUMENT_IO_COMMITTED_DURABILITY_UNCONFIRMED",
                    operation,
                    target = %target.display(),
                    "Atomic replacement doğrulandı ancak platform rename durability garantisi sunmuyor"
                );
                if let Some(backup) = outcome.recovery_backup.as_ref() {
                    tracing::warn!(
                        code = "DOCUMENT_IO_COMMITTED_BACKUP_RETAINED",
                        operation,
                        target = %target.display(),
                        recovery_path = %backup.display(),
                        reason = "durability_unconfirmed",
                        "Recovery backup otomatik silinmedi"
                    );
                }
            }
            Err(error) => {
                outcome.report.state = CommitState::NewCommittedDurabilityUnconfirmed;
                tracing::warn!(
                    code = "DOCUMENT_IO_COMMITTED_DURABILITY_UNCONFIRMED",
                    operation,
                    target = %target.display(),
                    error = %error,
                    "Atomic replacement committed but parent directory sync failed"
                );
                if let Some(backup) = outcome.recovery_backup.as_ref() {
                    tracing::warn!(
                        code = "DOCUMENT_IO_COMMITTED_BACKUP_RETAINED",
                        operation,
                        target = %target.display(),
                        recovery_path = %backup.display(),
                        reason = "directory_sync_failed",
                        "Recovery backup otomatik silinmedi"
                    );
                }
            }
        }
        return Ok(outcome.report);
    }

    unreachable!("bounded path-lock loop must return")
}

pub(crate) fn atomic_write_pdf<T, W, V>(
    operation: &str,
    source_paths: &[PathBuf],
    target: &Path,
    write: W,
    validate: V,
) -> Result<AtomicWriteReport, DocumentIoError>
where
    W: FnOnce(&Pdfium, &Path) -> Result<T, DocumentIoFailure>,
    V: FnOnce(&PdfDocument<'_>, &T) -> Result<(), DocumentIoFailure>,
{
    atomic_write_with(
        operation,
        source_paths,
        target,
        |temp| with_pdfium(|pdfium| write(pdfium, temp)),
        |temp, expected| {
            validate_general_pdf(temp)?;
            with_pdfium(|pdfium| {
                let document = pdfium.load_pdf_from_file(temp, None).map_err(|error| {
                    DocumentIoFailure::new(
                        DocumentIoStage::ValidateOpen,
                        format!("Temp PDF PDFium ile yeniden açılamadı: {error}"),
                    )
                })?;
                validate(&document, expected)
            })
        },
        |_| Ok(()),
    )
}

pub(crate) fn atomic_commit_prevalidated_pdf(
    operation: &str,
    source: &Path,
    target: &Path,
    worker_output: &Path,
    validated_digest: &str,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let source_path = source.to_path_buf();
    let worker_output_path = worker_output.to_path_buf();
    let worker_output_for_copy = worker_output_path.clone();
    let expected_digest = validated_digest.to_string();
    atomic_write_with(
        operation,
        &[source_path, worker_output_path],
        target,
        move |temp| {
            fs::copy(&worker_output_for_copy, temp).map_err(|error| {
                DocumentIoFailure::write(format!(
                    "Doğrulanmış worker çıktısı atomic temp'e kopyalanamadı: {error}"
                ))
            })?;
            Ok(())
        },
        move |temp, _| {
            validate_general_pdf(temp)?;
            let actual_digest = sha256_file(temp).map_err(|error| {
                DocumentIoFailure::invariant(format!(
                    "Atomic temp worker digest doğrulaması yapılamadı: {error}"
                ))
            })?;
            if actual_digest != expected_digest {
                return Err(DocumentIoFailure::invariant(
                    "Atomic temp, worker tarafından doğrulanan çıktıdan farklı",
                ));
            }
            Ok(())
        },
        |_| Ok(()),
    )
}

#[derive(Clone, Debug)]
pub(crate) enum TargetExpectation {
    Missing,
    Digest(String),
}

/// Secure-redaction publication has a stricter target contract than Save/Save
/// As. A digest alone cannot distinguish a same-content replacement from the
/// file the user originally selected, so bind the expectation to the canonical
/// namespace and filesystem identity captured at preflight.
#[derive(Clone, Debug)]
pub(crate) enum SecurePublicationTargetExpectation {
    Missing {
        namespace: PathBuf,
    },
    Existing {
        namespace: PathBuf,
        identity: FileIdentity,
        digest: String,
    },
}

/// Captures the canonical, hardened persistence identity used by a later
/// publication.  Callers must retain both return values: the resolved path is
/// the authoritative target and the expectation prevents a time-of-check to
/// time-of-use overwrite.
#[allow(dead_code)]
pub(crate) fn capture_target_expectation(
    target: &Path,
) -> Result<(PathBuf, TargetExpectation), String> {
    let resolved = resolve_target(target)?;
    let expectation = match resolved.snapshot.kind {
        SnapshotKind::Missing => TargetExpectation::Missing,
        SnapshotKind::RegularFile => TargetExpectation::Digest(
            sha256_file(&resolved.path)
                .map_err(|error| format!("Target SHA-256 okunamadı: {error}"))?,
        ),
    };
    Ok((resolved.path, expectation))
}

pub(crate) fn capture_secure_publication_target_expectation(
    target: &Path,
) -> Result<(PathBuf, SecurePublicationTargetExpectation), String> {
    let resolved = resolve_target(target)?;
    let expectation = match resolved.snapshot.kind {
        SnapshotKind::Missing => SecurePublicationTargetExpectation::Missing {
            namespace: resolved.namespace.clone(),
        },
        SnapshotKind::RegularFile => SecurePublicationTargetExpectation::Existing {
            namespace: resolved.namespace.clone(),
            identity: resolved.snapshot.identity.clone().ok_or_else(|| {
                format!(
                    "Secure publication target kimliği belirlenemedi: {}",
                    resolved.path.display()
                )
            })?,
            digest: sha256_file(&resolved.path)
                .map_err(|error| format!("Target SHA-256 okunamadı: {error}"))?,
        },
    };
    Ok((resolved.path, expectation))
}

/// Atomically publishes a worker artifact that was independently verified.
///
/// This deliberately does not open or parse the PDF. The worker/verifier own
/// PDF semantics; this boundary owns filesystem identity, byte digests and
/// durable replacement only.
pub(crate) fn atomic_publish_prevalidated_pdf(
    operation: &str,
    source: &Path,
    target: &Path,
    verified_artifact: &Path,
    expected_artifact_digest: &str,
    expectation: SecurePublicationTargetExpectation,
) -> Result<AtomicWriteReport, DocumentIoError> {
    atomic_publish_prevalidated_pdf_impl(
        operation,
        source,
        target,
        verified_artifact,
        expected_artifact_digest,
        expectation,
        false,
    )
}

/// Internal deterministic failure point used only by the secure-redaction
/// integration acceptance harness. Normal production callers always use the
/// wrapper above with corruption disabled.
#[cfg(feature = "secure-redaction-test-support")]
pub(crate) fn atomic_publish_prevalidated_pdf_for_acceptance(
    operation: &str,
    source: &Path,
    target: &Path,
    verified_artifact: &Path,
    expected_artifact_digest: &str,
    expectation: SecurePublicationTargetExpectation,
    corrupt_staged_temp: bool,
) -> Result<AtomicWriteReport, DocumentIoError> {
    atomic_publish_prevalidated_pdf_impl(
        operation,
        source,
        target,
        verified_artifact,
        expected_artifact_digest,
        expectation,
        corrupt_staged_temp,
    )
}

fn atomic_publish_prevalidated_pdf_impl(
    operation: &str,
    source: &Path,
    target: &Path,
    verified_artifact: &Path,
    expected_artifact_digest: &str,
    expectation: SecurePublicationTargetExpectation,
    corrupt_staged_temp: bool,
) -> Result<AtomicWriteReport, DocumentIoError> {
    #[cfg(not(feature = "secure-redaction-test-support"))]
    let _ = corrupt_staged_temp;
    let source_path = source.to_path_buf();
    let artifact_path = verified_artifact.to_path_buf();
    let artifact_for_copy = artifact_path.clone();
    let expected_digest = expected_artifact_digest.to_string();
    let expected_artifact_digest_for_copy = expected_digest.clone();
    let expected_target = expectation.clone();
    atomic_write_with(
        operation,
        &[source_path, artifact_path],
        target,
        move |temp| {
            // Hash the artifact immediately before copying: verifier success
            // never grants a mutable path authority.
            let artifact_digest = sha256_file(&artifact_for_copy).map_err(|error| {
                DocumentIoFailure::invariant(format!(
                    "Doğrulanmış artifact SHA-256 okunamadı: {error}"
                ))
            })?;
            if artifact_digest != expected_artifact_digest_for_copy {
                return Err(DocumentIoFailure::invariant(
                    "REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH: verifier sonrası artifact değişti",
                ));
            }
            fs::copy(&artifact_for_copy, temp).map_err(|error| {
                DocumentIoFailure::write(format!(
                    "Doğrulanmış artifact atomic temp'e kopyalanamadı: {error}"
                ))
            })?;
            #[cfg(feature = "secure-redaction-test-support")]
            if corrupt_staged_temp {
                use std::io::Write as _;
                let mut staged =
                    fs::OpenOptions::new()
                        .append(true)
                        .open(temp)
                        .map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Controlled staged-temp corruption açılamadı: {error}"
                            ))
                        })?;
                staged
                    .write_all(b"acceptance-corruption")
                    .map_err(|error| {
                        DocumentIoFailure::write(format!(
                            "Controlled staged-temp corruption yazılamadı: {error}"
                        ))
                    })?;
            }
            Ok(())
        },
        move |temp, _| {
            validate_general_pdf(temp)?;
            let temp_digest = sha256_file(temp).map_err(|error| {
                DocumentIoFailure::invariant(format!("Atomic temp SHA-256 okunamadı: {error}"))
            })?;
            if temp_digest != expected_digest {
                return Err(DocumentIoFailure::invariant(
                    "REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH: staged bytes verifier digest'iyle eşleşmiyor",
                ));
            }
            Ok(())
        },
        move |resolved_target| {
            let current = resolve_target(resolved_target).map_err(DocumentIoFailure::preflight)?;
            match &expected_target {
                SecurePublicationTargetExpectation::Missing { namespace } => {
                    if current.namespace != *namespace
                        || current.snapshot.kind != SnapshotKind::Missing
                    {
                        Err(DocumentIoFailure::preflight(
                            "REDACTION_TARGET_CONFLICT: hedef preflight sonrasında oluşturuldu",
                        ))
                    } else {
                        Ok(())
                    }
                }
                SecurePublicationTargetExpectation::Existing {
                    namespace,
                    identity,
                    digest,
                } => {
                    if current.namespace != *namespace
                        || current.snapshot.kind != SnapshotKind::RegularFile
                    {
                        return Err(DocumentIoFailure::preflight(
                            "REDACTION_TARGET_CONFLICT: mevcut hedef publication öncesi silindi",
                        ));
                    }
                    if current.snapshot.identity.as_ref() != Some(identity)
                        || current.snapshot.links.unwrap_or(1) > 1
                    {
                        return Err(DocumentIoFailure::preflight(
                        "REDACTION_TARGET_CONFLICT: hedef file identity preflight baseline'ından değişti",
                    ));
                    }
                    let actual = sha256_file(&current.path).map_err(|error| {
                        DocumentIoFailure::preflight(format!(
                            "Publication hedef SHA-256 okunamadı: {error}"
                        ))
                    })?;
                    if &actual == digest {
                        Ok(())
                    } else {
                        Err(DocumentIoFailure::preflight(
                            "REDACTION_TARGET_CONFLICT: hedef preflight baseline'ından değişti",
                        ))
                    }
                }
            }
        },
    )
}

pub(crate) fn atomic_copy_pdf(
    operation: &str,
    source: &Path,
    target: &Path,
    expectation: Option<TargetExpectation>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let source_path = source.to_path_buf();
    let source_for_copy = source_path.clone();
    atomic_write_with(
        operation,
        std::slice::from_ref(&source_path),
        target,
        move |temp| {
            fs::copy(&source_for_copy, temp).map_err(|error| {
                DocumentIoFailure::write(format!(
                    "Working PDF target temp'e kopyalanamadı: {error}"
                ))
            })?;
            Ok(())
        },
        |temp, _| {
            validate_general_pdf(temp)?;
            #[cfg(not(test))]
            {
                crate::job_worker::validate_pdf_file_isolated(
                    temp,
                    password,
                    Duration::from_secs(60),
                )
                .map_err(|error| {
                    DocumentIoFailure::new(
                        DocumentIoStage::ValidateOpen,
                        format!("Save candidate validation worker tarafından reddedildi: {error}"),
                    )
                })
            }
            #[cfg(test)]
            {
                with_pdfium(|pdfium| {
                    let document = pdfium
                        .load_pdf_from_file(temp, password.as_deref())
                        .map_err(|error| {
                            DocumentIoFailure::new(
                                DocumentIoStage::ValidateOpen,
                                format!("Save candidate PDFium ile açılamadı: {error}"),
                            )
                        })?;
                    if document.pages().is_empty() {
                        return Err(DocumentIoFailure::invariant(
                            "Save candidate en az bir sayfa içermeli",
                        ));
                    }
                    Ok(())
                })
            }
        },
        move |resolved_target| match &expectation {
            None => Ok(()),
            Some(TargetExpectation::Missing) => {
                if resolved_target.exists() {
                    Err(DocumentIoFailure::preflight(
                        "DOCUMENT_SESSION_CONFLICT: Save As hedefi seçimden sonra oluşturuldu",
                    ))
                } else {
                    Ok(())
                }
            }
            Some(TargetExpectation::Digest(expected)) => {
                if !resolved_target.exists() {
                    return Err(DocumentIoFailure::preflight(
                        "TARGET_NOT_FOUND: normal Save hedefi haricen silindi",
                    ));
                }
                let actual = sha256_file(resolved_target).map_err(|error| {
                    DocumentIoFailure::preflight(format!("Save target SHA-256 okunamadı: {error}"))
                })?;
                if &actual == expected {
                    Ok(())
                } else {
                    Err(DocumentIoFailure::preflight(
                        "DOCUMENT_SESSION_CONFLICT: Save target session baseline'ından değişmiş",
                    ))
                }
            }
        },
    )
}

#[cfg(unix)]
fn atomic_replace(
    operation: &str,
    temp: &mut TempOutput,
    target: &Path,
    _original: &FileSnapshot,
    _replacement: &FileSnapshot,
) -> Result<AtomicReplaceOutcome, DocumentIoError> {
    fs::rename(temp.path(), target).map_err(|error| {
        cleanup_failure(
            temp,
            DocumentIoError::new(
                operation,
                DocumentIoStage::Replace,
                CommitState::OriginalPreserved,
                error.to_string(),
            ),
        )
    })?;
    Ok(AtomicReplaceOutcome {
        report: AtomicWriteReport {
            state: CommitState::NewCommitted,
            committed_digest: String::new(),
        },
        recovery_backup: None,
    })
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsObservedFile {
    Missing,
    Original,
    Replacement,
    Other,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsReplaceClassification {
    OriginalPreserved,
    NewCommitted,
    RestoreOriginal,
    RecoveryRequired,
    Indeterminate,
}

#[cfg(any(windows, test))]
fn classify_windows_replace_state(
    target: WindowsObservedFile,
    backup: WindowsObservedFile,
) -> WindowsReplaceClassification {
    match (target, backup) {
        (WindowsObservedFile::Original, _) => WindowsReplaceClassification::OriginalPreserved,
        (WindowsObservedFile::Replacement, _) => WindowsReplaceClassification::NewCommitted,
        (WindowsObservedFile::Missing, WindowsObservedFile::Original) => {
            WindowsReplaceClassification::RestoreOriginal
        }
        (_, WindowsObservedFile::Original) => WindowsReplaceClassification::RecoveryRequired,
        _ => WindowsReplaceClassification::Indeterminate,
    }
}

#[cfg(windows)]
fn atomic_replace(
    operation: &str,
    temp: &mut TempOutput,
    target: &Path,
    original: &FileSnapshot,
    replacement: &FileSnapshot,
) -> Result<AtomicReplaceOutcome, DocumentIoError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_UNABLE_TO_MOVE_REPLACEMENT, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
        ERROR_UNABLE_TO_REMOVE_REPLACED,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn recovery_path(parent: &Path) -> PathBuf {
        loop {
            let candidate = parent.join(format!(
                ".antigravity-recovery-{}.pdf.bak",
                uuid::Uuid::new_v4()
            ));
            if !candidate.exists() {
                return candidate;
            }
        }
    }

    fn observe(
        path: &Path,
        original: &FileSnapshot,
        replacement: &FileSnapshot,
    ) -> WindowsObservedFile {
        match snapshot_regular_file(path, false) {
            Ok(snapshot) if snapshot.kind == SnapshotKind::Missing => WindowsObservedFile::Missing,
            Ok(snapshot) if snapshot.identity == original.identity => WindowsObservedFile::Original,
            Ok(snapshot) if snapshot.identity == replacement.identity => {
                WindowsObservedFile::Replacement
            }
            Ok(_) | Err(_) => WindowsObservedFile::Other,
        }
    }

    fn error_detail(code: u32, error: &std::io::Error) -> String {
        let documented = match code {
            ERROR_UNABLE_TO_REMOVE_REPLACED => "ERROR_UNABLE_TO_REMOVE_REPLACED(1175)",
            ERROR_UNABLE_TO_MOVE_REPLACEMENT => "ERROR_UNABLE_TO_MOVE_REPLACEMENT(1176)",
            ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 => "ERROR_UNABLE_TO_MOVE_REPLACEMENT_2(1177)",
            _ => "OTHER_REPLACEFILE_ERROR",
        };
        format!("{documented}: {error}")
    }

    fn preserve_temp_path(temp: &mut TempOutput, error: &mut DocumentIoError) {
        match temp.preserve() {
            Ok(path) => error
                .detail
                .push_str(&format!("; validated_replacement_path={}", path.display())),
            Err(preserve_error) => error.detail.push_str(&format!(
                "; validated_replacement_preserve_failed={preserve_error}"
            )),
        }
    }

    let target_wide = wide(target);
    let temp_wide = wide(temp.path());
    if original.kind == SnapshotKind::Missing {
        let moved = unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved != 0 {
            return Ok(AtomicReplaceOutcome {
                report: AtomicWriteReport {
                    state: CommitState::NewCommittedDurabilityUnconfirmed,
                    committed_digest: String::new(),
                },
                recovery_backup: None,
            });
        }

        let os_error = std::io::Error::last_os_error();
        return match observe(target, original, replacement) {
            WindowsObservedFile::Replacement => Ok(AtomicReplaceOutcome {
                report: AtomicWriteReport {
                    state: CommitState::NewCommittedDurabilityUnconfirmed,
                    committed_digest: String::new(),
                },
                recovery_backup: None,
            }),
            WindowsObservedFile::Missing => Err(cleanup_failure(
                temp,
                DocumentIoError::new(
                    operation,
                    DocumentIoStage::Replace,
                    CommitState::OriginalPreserved,
                    os_error.to_string(),
                ),
            )),
            _ => {
                let mut error = DocumentIoError::new(
                    operation,
                    DocumentIoStage::Replace,
                    CommitState::Indeterminate,
                    os_error.to_string(),
                );
                preserve_temp_path(temp, &mut error);
                Err(error)
            }
        };
    }

    let parent = target.parent().expect("resolved target has parent");
    let backup = recovery_path(parent);
    let backup_wide = wide(&backup);
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            temp_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced != 0 {
        return Ok(AtomicReplaceOutcome {
            report: AtomicWriteReport {
                state: CommitState::NewCommittedDurabilityUnconfirmed,
                committed_digest: String::new(),
            },
            recovery_backup: backup.exists().then_some(backup),
        });
    }

    let last_error = unsafe { GetLastError() };
    let os_error = std::io::Error::from_raw_os_error(last_error as i32);
    let detail = error_detail(last_error, &os_error);
    let target_state = observe(target, original, replacement);
    let backup_state = observe(&backup, original, replacement);

    match classify_windows_replace_state(target_state, backup_state) {
        WindowsReplaceClassification::NewCommitted => Ok(AtomicReplaceOutcome {
            report: AtomicWriteReport {
                state: CommitState::NewCommittedDurabilityUnconfirmed,
                committed_digest: String::new(),
            },
            recovery_backup: backup.exists().then_some(backup),
        }),
        WindowsReplaceClassification::OriginalPreserved => {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::Replace,
                CommitState::OriginalPreserved,
                detail,
            );
            if backup.exists() {
                error = error.with_recovery_path(backup);
            }
            Err(cleanup_failure(temp, error))
        }
        WindowsReplaceClassification::RestoreOriginal => {
            let restored = unsafe {
                MoveFileExW(
                    backup_wide.as_ptr(),
                    target_wide.as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            };
            if restored != 0
                && observe(target, original, replacement) == WindowsObservedFile::Original
            {
                return Err(cleanup_failure(
                    temp,
                    DocumentIoError::new(
                        operation,
                        DocumentIoStage::Recover,
                        CommitState::OriginalRecovered,
                        format!("{detail}; original recovery backup'tan geri yüklendi"),
                    ),
                ));
            }

            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::Recover,
                CommitState::RecoveryRequired,
                format!(
                    "{detail}; original geri yüklenemedi: {}",
                    std::io::Error::last_os_error()
                ),
            );
            if backup.exists() {
                error = error.with_recovery_path(backup);
            }
            preserve_temp_path(temp, &mut error);
            Err(error)
        }
        WindowsReplaceClassification::RecoveryRequired => {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::Recover,
                CommitState::RecoveryRequired,
                format!("{detail}; bilinmeyen target üzerine recovery uygulanmadı"),
            )
            .with_recovery_path(backup);
            preserve_temp_path(temp, &mut error);
            Err(error)
        }
        WindowsReplaceClassification::Indeterminate => {
            let mut error = DocumentIoError::new(
                operation,
                DocumentIoStage::Replace,
                CommitState::Indeterminate,
                detail,
            );
            if backup.exists() {
                error = error.with_recovery_path(backup);
            }
            preserve_temp_path(temp, &mut error);
            Err(error)
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn atomic_replace(
    operation: &str,
    temp: &mut TempOutput,
    _target: &Path,
    _original: &FileSnapshot,
    _replacement: &FileSnapshot,
) -> Result<AtomicReplaceOutcome, DocumentIoError> {
    Err(cleanup_failure(
        temp,
        DocumentIoError::new(
            operation,
            DocumentIoStage::Replace,
            CommitState::OriginalPreserved,
            "Bu platform için atomic replace uygulanmadı",
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    fn temp_artifacts(parent: &Path) -> Vec<PathBuf> {
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(TEMP_PREFIX))
            })
            .collect()
    }

    #[test]
    fn document_io_success_replaces_existing_target_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        fs::write(&target, b"original").unwrap();

        let report = atomic_write_with(
            "test_success",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |temp, _| {
                if fs::read(temp).unwrap() == b"replacement" {
                    Ok(())
                } else {
                    Err(DocumentIoFailure::invariant("unexpected bytes"))
                }
            },
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(report.state, CommitState::NewCommitted);
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_success_creates_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let target = dir.path().join("new-target.pdf");
        fs::write(&source, b"source").unwrap();

        atomic_write_with(
            "test_create",
            std::slice::from_ref(&source),
            &target,
            |temp| {
                fs::write(temp, b"new-target").unwrap();
                Ok(())
            },
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new-target");
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn persistence_target_identity_reuses_atomic_path_authority() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        fs::write(&target, b"target").unwrap();

        assert!(same_persistence_target(&target, &target).unwrap());
        assert!(!same_persistence_target(&target, &dir.path().join("other.pdf")).unwrap());
    }

    #[test]
    fn document_io_normalizes_dot_alias_for_same_source_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        let source_alias = dir.path().join(".").join("target.pdf");
        fs::write(&target, b"original").unwrap();

        atomic_write_with(
            "test_path_alias",
            std::slice::from_ref(&source_alias),
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(fs::read(target).unwrap(), b"replacement");
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_missing_parent_is_a_preflight_error() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("missing-parent").join("target.pdf");

        let error = atomic_write_with(
            "test_missing_parent",
            &[],
            &target,
            |_temp| Ok(()),
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("parent"));
        assert!(!target.exists());
    }

    #[test]
    fn document_io_rejects_directory_target() {
        let dir = tempfile::tempdir().unwrap();

        let error = atomic_write_with(
            "test_directory_target",
            &[],
            dir.path(),
            |_temp| Ok(()),
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("regular file"));
    }

    #[test]
    fn document_io_simulated_write_failure_preserves_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        let original = b"original-byte-for-byte".to_vec();
        fs::write(&target, &original).unwrap();

        let error = atomic_write_with(
            "test_write_failure",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"partial").unwrap();
                Err::<(), _>(DocumentIoFailure::write("simulated save failure"))
            },
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.state, CommitState::OriginalPreserved);
        assert!(error
            .to_string()
            .contains("operation=test_write_failure stage=write_pdf"));
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_validation_failure_preserves_original_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        let original = b"original".to_vec();
        fs::write(&target, &original).unwrap();

        let error = atomic_write_with(
            "test_validation_failure",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"invalid").unwrap();
                Ok(())
            },
            |_temp, _| Err(DocumentIoFailure::invariant("simulated validation failure")),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::ValidateInvariant);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_detects_external_target_change_before_commit() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        fs::write(&target, b"original").unwrap();

        let error = atomic_write_with(
            "test_external_change",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |_temp, _| {
                fs::write(&target, b"external-change").unwrap();
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::RevalidatePaths);
        assert_eq!(fs::read(&target).unwrap(), b"external-change");
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_same_target_writers_are_serialized() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        fs::write(&target, b"original").unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let handles = (0..2)
            .map(|index| {
                let target = target.clone();
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                thread::spawn(move || {
                    atomic_write_with(
                        "test_concurrency",
                        std::slice::from_ref(&target),
                        &target,
                        |temp| {
                            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(now, Ordering::SeqCst);
                            thread::yield_now();
                            let mut file = File::create(temp).unwrap();
                            write!(file, "writer-{index}").unwrap();
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        },
                        |_temp, _| Ok(()),
                        |_| Ok(()),
                    )
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_same_missing_target_writers_share_one_namespace_lock() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new-target.pdf");
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let successful = Arc::new(AtomicUsize::new(0));

        let handles = (0..2)
            .map(|index| {
                let target = target.clone();
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                let successful = Arc::clone(&successful);
                thread::spawn(move || {
                    let result = atomic_write_with(
                        "test_missing_target_concurrency",
                        &[],
                        &target,
                        |temp| {
                            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(now, Ordering::SeqCst);
                            thread::yield_now();
                            let mut file = File::create(temp).unwrap();
                            write!(file, "writer-{index}").unwrap();
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        },
                        |_temp, _| Ok(()),
                        |resolved| {
                            if resolved.exists() {
                                Err(DocumentIoFailure::preflight(
                                    "missing-target namespace was claimed",
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    );
                    if result.is_ok() {
                        successful.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        assert_eq!(successful.load(Ordering::SeqCst), 1);
        assert!(target.exists());
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_rejects_existing_target_with_multiple_hard_links() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        let alias = dir.path().join("alias.pdf");
        fs::write(&target, b"original").unwrap();
        fs::hard_link(&target, &alias).unwrap();

        let error = atomic_write_with(
            "test_hard_link",
            std::slice::from_ref(&target),
            &target,
            |_temp| Ok(()),
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("hard-link"));
    }

    #[cfg(unix)]
    #[test]
    fn document_io_preserves_existing_unix_permission_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        fs::write(&target, b"original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write_with(
            "test_permissions",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn document_io_creates_new_unix_target_with_explicit_private_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new-target.pdf");

        atomic_write_with(
            "test_new_target_permissions",
            &[],
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn document_io_rejects_symlink_output_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.pdf");
        let target = dir.path().join("target.pdf");
        fs::write(&real, b"original").unwrap();
        symlink(&real, &target).unwrap();

        let error = atomic_write_with(
            "test_symlink",
            std::slice::from_ref(&real),
            &target,
            |_temp| Ok(()),
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("Sembolik link"));
        assert_eq!(fs::read(&real).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn document_io_rejects_dangling_symlink_output_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.pdf");
        let target = dir.path().join("dangling-target.pdf");
        symlink(&missing, &target).unwrap();

        let error = atomic_write_with(
            "test_dangling_symlink",
            &[],
            &target,
            |_temp| Ok(()),
            |_temp, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("Sembolik link"));
        assert!(!missing.exists());
    }

    #[cfg(windows)]
    #[test]
    fn document_io_rejects_windows_symlink_output_target() {
        use std::os::windows::fs::symlink_file;

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.pdf");
        let target = dir.path().join("target.pdf");
        fs::write(&real, b"original").unwrap();
        symlink_file(&real, &target).unwrap();

        let error = atomic_write_with(
            "test_windows_symlink",
            std::slice::from_ref(&real),
            &target,
            |_temp| Ok(()),
            |_temp, _| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.to_string().contains("Sembolik link"));
        assert_eq!(fs::read(&real).unwrap(), b"original");
    }

    #[cfg(windows)]
    #[test]
    fn document_io_windows_sharing_violation_preserves_original() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.pdf");
        let original = b"original-byte-for-byte".to_vec();
        fs::write(&target, &original).unwrap();
        let exclusive_handle = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();

        let error = atomic_write_with(
            "test_windows_sharing_violation",
            std::slice::from_ref(&target),
            &target,
            |temp| {
                fs::write(temp, b"replacement").unwrap();
                Ok(())
            },
            |_temp, _| Ok(()),
        )
        .unwrap_err();

        drop(exclusive_handle);
        assert_eq!(error.stage, DocumentIoStage::Replace);
        assert_eq!(error.state, CommitState::OriginalPreserved);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(temp_artifacts(dir.path()).is_empty());
    }

    #[test]
    fn document_io_windows_state_classification_uses_observed_filesystem_state() {
        use WindowsObservedFile::{Missing, Original, Other, Replacement};
        use WindowsReplaceClassification::{
            Indeterminate, NewCommitted, OriginalPreserved, RecoveryRequired, RestoreOriginal,
        };

        assert_eq!(
            classify_windows_replace_state(Original, Other),
            OriginalPreserved
        );
        assert_eq!(
            classify_windows_replace_state(Replacement, Original),
            NewCommitted
        );
        assert_eq!(
            classify_windows_replace_state(Missing, Original),
            RestoreOriginal
        );
        assert_eq!(
            classify_windows_replace_state(Other, Original),
            RecoveryRequired
        );
        assert_eq!(
            classify_windows_replace_state(Missing, Missing),
            Indeterminate
        );
        assert_eq!(
            classify_windows_replace_state(Other, Replacement),
            Indeterminate
        );
    }

    #[test]
    fn test_windows_dacl_negative_rejects_broad_untrusted_access() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dacl_test.pdf");
        fs::write(&target, b"test").unwrap();

        let result = validate_windows_dacl_security(&target);
        // Returns Ok(()) on standard OS permission or UNSAFE_WINDOWS_DACL error if NULL DACL
        if let Err(err) = result {
            assert!(err.contains("UNSAFE_WINDOWS_DACL"));
        }
    }
}
#[test]
fn prevalidated_worker_output_commits_only_the_digest_bound_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let working = directory.path().join("working.pdf");
    let worker_output = directory.path().join("worker-output.pdf");
    fs::write(&working, b"%PDF-original").unwrap();
    fs::write(&worker_output, b"%PDF-worker-output").unwrap();
    let digest = sha256_file(&worker_output).unwrap();

    let report = atomic_commit_prevalidated_pdf(
        "isolated_mutation_test",
        &working,
        &working,
        &worker_output,
        &digest,
    )
    .unwrap();

    assert!(matches!(
        report.state,
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed
    ));
    assert_eq!(fs::read(&working).unwrap(), b"%PDF-worker-output");
}

#[test]
fn prevalidated_worker_output_digest_mismatch_preserves_original() {
    let directory = tempfile::tempdir().unwrap();
    let working = directory.path().join("working.pdf");
    let worker_output = directory.path().join("worker-output.pdf");
    fs::write(&working, b"%PDF-original").unwrap();
    fs::write(&worker_output, b"%PDF-worker-output").unwrap();

    let error = atomic_commit_prevalidated_pdf(
        "isolated_mutation_test",
        &working,
        &working,
        &worker_output,
        "not-the-worker-digest",
    )
    .unwrap_err();

    assert_eq!(error.stage, DocumentIoStage::ValidateInvariant);
    assert_eq!(error.state, CommitState::OriginalPreserved);
    assert_eq!(fs::read(&working).unwrap(), b"%PDF-original");
}

#[test]
fn verified_publication_binds_missing_target_to_verified_digest() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    let artifact = directory.path().join("verified-artifact.pdf");
    let target = directory.path().join("published.pdf");
    fs::write(&snapshot, b"%PDF-snapshot").unwrap();
    fs::write(&artifact, b"%PDF-verified-artifact").unwrap();
    let digest = sha256_file(&artifact).unwrap();
    let (canonical_target, expectation) =
        capture_secure_publication_target_expectation(&target).unwrap();

    let report = atomic_publish_prevalidated_pdf(
        "secure_redaction_publish_test",
        &snapshot,
        &canonical_target,
        &artifact,
        &digest,
        expectation,
    )
    .unwrap();

    assert!(matches!(
        report.state,
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed
    ));
    assert_eq!(report.committed_digest, digest);
    assert_eq!(sha256_file(&target).unwrap(), digest);
}

#[test]
fn verified_publication_rejects_target_baseline_race_without_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    let artifact = directory.path().join("verified-artifact.pdf");
    let target = directory.path().join("published.pdf");
    fs::write(&snapshot, b"%PDF-snapshot").unwrap();
    fs::write(&artifact, b"%PDF-verified-artifact").unwrap();
    fs::write(&target, b"%PDF-original-target").unwrap();
    let digest = sha256_file(&artifact).unwrap();
    let (canonical_target, expectation) =
        capture_secure_publication_target_expectation(&target).unwrap();
    fs::write(&target, b"%PDF-racing-writer").unwrap();

    let error = atomic_publish_prevalidated_pdf(
        "secure_redaction_publish_test",
        &snapshot,
        &canonical_target,
        &artifact,
        &digest,
        expectation,
    )
    .unwrap_err();

    assert_eq!(error.state, CommitState::OriginalPreserved);
    assert_eq!(fs::read(&target).unwrap(), b"%PDF-racing-writer");
}

#[cfg(unix)]
#[test]
fn verified_publication_rejects_same_digest_identity_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    let artifact = directory.path().join("verified-artifact.pdf");
    let target = directory.path().join("published.pdf");
    let replacement = directory.path().join("replacement.pdf");
    fs::write(&snapshot, b"%PDF-snapshot").unwrap();
    fs::write(&artifact, b"%PDF-verified-artifact").unwrap();
    fs::write(&target, b"%PDF-same-bytes").unwrap();
    let digest = sha256_file(&artifact).unwrap();
    let (canonical_target, expectation) =
        capture_secure_publication_target_expectation(&target).unwrap();
    fs::write(&replacement, b"%PDF-same-bytes").unwrap();
    fs::rename(&replacement, &target).unwrap();

    let error = atomic_publish_prevalidated_pdf(
        "secure_redaction_publish_test",
        &snapshot,
        &canonical_target,
        &artifact,
        &digest,
        expectation,
    )
    .unwrap_err();

    assert_eq!(error.state, CommitState::OriginalPreserved);
    assert_eq!(fs::read(&target).unwrap(), b"%PDF-same-bytes");
}
