//! Batch inputs: folder scanning and per-file admission.
//!
//! Only file-system metadata is read here, in the main process. A PDF's bytes
//! are first read by `copy_source_consistent` into the private batch directory
//! and parsed only by the job worker.

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const MAX_BATCH_FILES: usize = 500;
/// Same bound as the structure/optimizer input limit and the worker's
/// credential document limit.
pub(crate) const MAX_BATCH_FILE_BYTES: u64 =
    crate::document_structure::MAX_STRUCTURE_INPUT_BYTES as u64;
pub(crate) const MAX_BATCH_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_SCAN_DEPTH: usize = 8;
/// Directory entries a scan may visit, so a huge tree cannot stall the command.
const MAX_SCAN_ENTRIES: usize = 50_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInputFile {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReason {
    pub code: String,
    pub message: String,
}

impl BatchReason {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInputRejection {
    pub path: String,
    #[serde(flatten)]
    pub reason: BatchReason,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInputScan {
    pub files: Vec<BatchInputFile>,
    pub rejected: Vec<BatchInputRejection>,
    /// More PDFs were found than a batch may take; `files` holds the first ones.
    pub truncated: bool,
}

/// Directories no batch input or output may live in: the session/job temp
/// root and the app data directory.
#[derive(Clone, Debug, Default)]
pub(crate) struct PrivateRoots(Vec<PathBuf>);

impl PrivateRoots {
    pub(crate) fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self(roots.into_iter().collect())
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.0
            .iter()
            .any(|root| crate::document_session::reject_private_target(path, root).is_err())
    }
}

/// An input admitted for processing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedInput {
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
    pub(crate) identity: String,
}

fn has_pdf_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Checks one input file. The returned path is canonical; the identity lets
/// the runner drop duplicates and refuse to publish over any input.
pub(crate) fn admit_input_file(
    path: &Path,
    private: &PrivateRoots,
) -> Result<AdmittedInput, BatchReason> {
    let link_metadata = fs::symlink_metadata(path).map_err(|error| {
        BatchReason::new("BATCH_INPUT_NOT_FOUND", format!("Dosya okunamadı: {error}"))
    })?;
    if is_link(&link_metadata) {
        return Err(BatchReason::new(
            "BATCH_INPUT_SYMLINK",
            "Sembolik bağlar toplu işleme alınmaz",
        ));
    }
    if !link_metadata.is_file() {
        return Err(BatchReason::new(
            "BATCH_INPUT_NOT_FILE",
            "Yol normal bir dosya değil",
        ));
    }
    if !has_pdf_extension(path) {
        return Err(BatchReason::new(
            "BATCH_INPUT_NOT_PDF",
            "Yalnız .pdf uzantılı dosyalar işlenir",
        ));
    }
    let size = link_metadata.len();
    if size == 0 {
        return Err(BatchReason::new("BATCH_INPUT_EMPTY", "Dosya boş"));
    }
    if size > MAX_BATCH_FILE_BYTES {
        return Err(BatchReason::new(
            "BATCH_INPUT_TOO_LARGE",
            format!(
                "Dosya toplu işlem sınırını aşıyor ({} MiB)",
                MAX_BATCH_FILE_BYTES / (1024 * 1024)
            ),
        ));
    }
    let links = crate::document_io::regular_file_link_count(path).map_err(|error| {
        BatchReason::new(
            "BATCH_INPUT_NOT_FOUND",
            format!("Dosya bağlantı sayısı okunamadı: {error}"),
        )
    })?;
    if links > 1 {
        return Err(BatchReason::new(
            "BATCH_INPUT_HARD_LINK",
            "Birden fazla adı olan (hard link) dosyalar toplu işleme alınmaz",
        ));
    }
    let canonical = path.canonicalize().map_err(|error| {
        BatchReason::new(
            "BATCH_INPUT_NOT_FOUND",
            format!("Dosya yolu çözümlenemedi: {error}"),
        )
    })?;
    if private.contains(&canonical) {
        return Err(BatchReason::new(
            "BATCH_INPUT_PRIVATE_PATH",
            "Uygulamanın özel alanındaki dosyalar işlenmez",
        ));
    }
    let identity = crate::document_session::source_identity(&canonical)
        .map_err(|error| BatchReason::new(error.code, error.message))?;
    Ok(AdmittedInput {
        path: canonical,
        size,
        identity,
    })
}

struct ScanState<'a> {
    private: &'a PrivateRoots,
    scan: BatchInputScan,
    seen: HashSet<PathBuf>,
    visited: usize,
}

impl ScanState<'_> {
    fn full(&self) -> bool {
        self.scan.truncated
    }

    fn add_file(&mut self, path: &Path) {
        match admit_input_file(path, self.private) {
            Ok(input) => {
                if !self.seen.insert(input.path.clone()) {
                    return;
                }
                if self.scan.files.len() >= MAX_BATCH_FILES {
                    self.scan.truncated = true;
                    return;
                }
                self.scan.files.push(BatchInputFile {
                    path: input.path.to_string_lossy().into_owned(),
                    size: input.size,
                });
            }
            Err(reason) => self.scan.rejected.push(BatchInputRejection {
                path: path.to_string_lossy().into_owned(),
                reason,
            }),
        }
    }

    fn walk(&mut self, directory: &Path, depth: usize, recursive: bool) {
        let mut entries = match fs::read_dir(directory) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .collect::<Vec<_>>(),
            Err(error) => {
                self.scan.rejected.push(BatchInputRejection {
                    path: directory.to_string_lossy().into_owned(),
                    reason: BatchReason::new(
                        "BATCH_INPUT_DIRECTORY_UNREADABLE",
                        format!("Klasör okunamadı: {error}"),
                    ),
                });
                return;
            }
        };
        entries.sort();
        for entry in entries {
            if self.full() {
                return;
            }
            self.visited += 1;
            if self.visited > MAX_SCAN_ENTRIES {
                self.scan.truncated = true;
                return;
            }
            let Ok(metadata) = fs::symlink_metadata(&entry) else {
                continue;
            };
            if metadata.is_dir() && !is_link(&metadata) {
                if recursive && depth < MAX_SCAN_DEPTH {
                    self.walk(&entry, depth + 1, recursive);
                }
            } else if has_pdf_extension(&entry) {
                // Symlinked or otherwise unusable PDFs are listed as rejected so the
                // user sees why they were left out; other files are ignored.
                self.add_file(&entry);
            }
        }
    }
}

/// Expands the user's selection into PDF files. Folders are read one level
/// deep unless `recursive`; symlinked folders are never followed.
pub(crate) fn scan_inputs(
    paths: &[String],
    recursive: bool,
    private: &PrivateRoots,
) -> BatchInputScan {
    let mut state = ScanState {
        private,
        scan: BatchInputScan::default(),
        seen: HashSet::new(),
        visited: 0,
    };
    for raw in paths {
        if state.full() {
            break;
        }
        let path = PathBuf::from(raw);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
                if private.contains(&path) {
                    state.scan.rejected.push(BatchInputRejection {
                        path: raw.clone(),
                        reason: BatchReason::new(
                            "BATCH_INPUT_PRIVATE_PATH",
                            "Uygulamanın özel alanı taranmaz",
                        ),
                    });
                } else {
                    state.walk(&path, 0, recursive);
                }
            }
            _ => state.add_file(&path),
        }
    }
    state.scan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pdf() -> Vec<u8> {
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/minimal.pdf"))
            .unwrap()
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn admits_a_regular_pdf_with_a_canonical_path_and_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rapor.PDF");
        write(&path, &minimal_pdf());
        let input = admit_input_file(&path, &PrivateRoots::default()).unwrap();
        assert_eq!(input.path, path.canonicalize().unwrap());
        assert!(!input.identity.is_empty());
    }

    #[test]
    fn rejects_wrong_extension_empty_files_and_directories() {
        let directory = tempfile::tempdir().unwrap();
        let text = directory.path().join("not.txt");
        write(&text, b"%PDF-1.4");
        let empty = directory.path().join("empty.pdf");
        write(&empty, b"");
        let folder = directory.path().join("folder.pdf");
        fs::create_dir(&folder).unwrap();
        let private = PrivateRoots::default();
        assert_eq!(
            admit_input_file(&text, &private).unwrap_err().code,
            "BATCH_INPUT_NOT_PDF"
        );
        assert_eq!(
            admit_input_file(&empty, &private).unwrap_err().code,
            "BATCH_INPUT_EMPTY"
        );
        assert_eq!(
            admit_input_file(&folder, &private).unwrap_err().code,
            "BATCH_INPUT_NOT_FILE"
        );
        assert_eq!(
            admit_input_file(&directory.path().join("missing.pdf"), &private)
                .unwrap_err()
                .code,
            "BATCH_INPUT_NOT_FOUND"
        );
    }

    #[test]
    fn rejects_files_over_the_size_limit_without_reading_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("big.pdf");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_BATCH_FILE_BYTES + 1).unwrap();
        assert_eq!(
            admit_input_file(&path, &PrivateRoots::default())
                .unwrap_err()
                .code,
            "BATCH_INPUT_TOO_LARGE"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_hard_links() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real.pdf");
        write(&real, &minimal_pdf());
        let link = directory.path().join("link.pdf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let private = PrivateRoots::default();
        assert_eq!(
            admit_input_file(&link, &private).unwrap_err().code,
            "BATCH_INPUT_SYMLINK"
        );

        let alias = directory.path().join("alias.pdf");
        fs::hard_link(&real, &alias).unwrap();
        assert_eq!(
            admit_input_file(&real, &private).unwrap_err().code,
            "BATCH_INPUT_HARD_LINK"
        );
        assert_eq!(
            admit_input_file(&alias, &private).unwrap_err().code,
            "BATCH_INPUT_HARD_LINK"
        );
    }

    #[test]
    fn rejects_files_inside_a_private_root() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private").join("x.pdf");
        write(&path, &minimal_pdf());
        let private = PrivateRoots::new([directory.path().join("private")]);
        assert_eq!(
            admit_input_file(&path, &private).unwrap_err().code,
            "BATCH_INPUT_PRIVATE_PATH"
        );
    }

    #[test]
    fn scan_reads_one_level_unless_recursive_and_skips_other_files() {
        let directory = tempfile::tempdir().unwrap();
        let pdf = minimal_pdf();
        write(&directory.path().join("b.pdf"), &pdf);
        write(&directory.path().join("a.pdf"), &pdf);
        write(&directory.path().join("notes.txt"), b"x");
        write(&directory.path().join("sub").join("c.pdf"), &pdf);
        let root = directory.path().to_string_lossy().into_owned();

        let shallow = scan_inputs(std::slice::from_ref(&root), false, &PrivateRoots::default());
        let names: Vec<_> = shallow
            .files
            .iter()
            .map(|file| Path::new(&file.path).file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.pdf", "b.pdf"]);
        assert!(shallow.rejected.is_empty());

        let deep = scan_inputs(&[root], true, &PrivateRoots::default());
        assert_eq!(deep.files.len(), 3);
    }

    #[test]
    fn scan_lists_a_file_given_twice_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.pdf");
        write(&path, &minimal_pdf());
        let raw = path.to_string_lossy().into_owned();
        let scan = scan_inputs(&[raw.clone(), raw], false, &PrivateRoots::default());
        assert_eq!(scan.files.len(), 1);
    }

    #[test]
    fn scan_stops_at_the_file_limit_and_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let pdf = minimal_pdf();
        for index in 0..=MAX_BATCH_FILES {
            write(&directory.path().join(format!("f{index:04}.pdf")), &pdf);
        }
        let scan = scan_inputs(
            &[directory.path().to_string_lossy().into_owned()],
            false,
            &PrivateRoots::default(),
        );
        assert_eq!(scan.files.len(), MAX_BATCH_FILES);
        assert!(scan.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_symlinked_folders() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("x.pdf"), &minimal_pdf());
        std::os::unix::fs::symlink(outside.path(), directory.path().join("linked")).unwrap();
        let scan = scan_inputs(
            &[directory.path().to_string_lossy().into_owned()],
            true,
            &PrivateRoots::default(),
        );
        assert!(scan.files.is_empty());
    }
}
