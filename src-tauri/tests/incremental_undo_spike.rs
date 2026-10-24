//! Faz 0 spike (docs/engineering/undo-redo-and-incremental-save-design.md §8 S1–S6, §9.1 K5):
//! PDFium `FPDF_INCREMENTAL` ve lopdf `IncrementalDocument` sayfa/annotation mutasyonlarında
//! append-only ve geçerli çıktı üretiyor mu; xref onarımı tespit edilebiliyor mu; geçmiş için
//! hard link tutmak atomik rename ile uyumlu mu; maliyetler ne. Atılabilir ölçüm kodu;
//! production yolu değildir ve production modüllerini çağırmaz.
//!
//! Çalıştırma (src-tauri içinden):
//! `SPIKE_OUT=/tmp/x TAURI_RESOURCE_DIR=$PWD cargo test --test incremental_undo_spike -- --ignored --nocapture --test-threads=1`
//! İsteğe bağlı: `SPIKE_EXTRA="yol::parola;yol::"` ek girdiler, `SPIKE_PERF="yol;yol"` S6/S6b
//! girdileri, `SPIKE_K5=yol` görselli girdi (K5 deneyi), `SPIKE_ONE=yol` + `SPIKE_MODE=full|pdfium-only`
//! (K5-B maliyeti; tepe bellek için `/usr/bin/time -l` altında ayrı süreçte çalıştırılır).
//! Release ölçümleri: release profili `panic = "abort"` olduğu için test harness'ı
//! `cargo test --release --config 'profile.release.panic="unwind"'` ister.
//! Perf girdileri `scripts/generate-perf-fixtures.py` ile üretilir ve commit edilmez.

use lopdf::xref::XrefEntry;
use lopdf::{Document, IncrementalDocument, Object};
use pdfium_render::prelude::*;
use preview_alternative_lib::get_pdfium;
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::fmt::Write as _;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};
use std::time::Instant;

const FPDF_INCREMENTAL: FPDF_DWORD = 1;
const FPDF_FULL_REWRITE: FPDF_DWORD = 0;
const MARK: &str = "SPIKEMARK7";
const ANNOT_TEXT: FPDF_ANNOTATION_SUBTYPE = 1;
const COLORTYPE_COLOR: FPDFANNOT_COLORTYPE = 0;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var("SPIKE_OUT").expect("SPIKE_OUT gerekli"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha(bytes: &[u8]) -> String {
    let mut out = String::new();
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn find_last(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).rposition(|w| w == n)
}

fn find_first(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn number_after(h: &[u8], at: usize) -> Option<u64> {
    let digits: Vec<u8> = h[at..]
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

fn last_startxref(bytes: &[u8]) -> Option<(usize, u64)> {
    let at = find_last(bytes, b"startxref")?;
    Some((at, number_after(bytes, at + b"startxref".len())?))
}

/// Girdi listesi: (etiket, yol, parola).
fn inputs() -> Vec<(String, PathBuf, Option<String>)> {
    let f = fixtures();
    let mut list: Vec<(String, PathBuf, Option<String>)> = [
        ("table-1p", "pdf/minimal.pdf", None),
        ("table-3p", "pdf/three-distinct-pages.pdf", None),
        ("table-annot", "pdf/annotations.pdf", None),
        ("table-rich", "pdf/reorder-rich.pdf", None),
        (
            "table-incremental",
            "sanitization/incremental-revision.pdf",
            None,
        ),
        (
            "xrefstream-objstm",
            "sanitization/object-xref-stream.pdf",
            None,
        ),
        ("xrefstream-objstm-2", "redaction/object-stream.pdf", None),
        (
            "xrefstream-chain",
            "sanitization/incremental-xref-stream-chain.pdf",
            None,
        ),
        ("hybrid", "sanitization/hybrid-xref.pdf", None),
        ("signature", "sanitization/signature.pdf", None),
        (
            "signature-byterange",
            "redaction/signature-marked-byterange.pdf",
            None,
        ),
        (
            "encrypted-rc4",
            "pdf/encrypted.pdf",
            Some("fixture-password"),
        ),
    ]
    .into_iter()
    .map(|(l, p, pw)| (l.to_string(), f.join(p), pw.map(str::to_string)))
    .collect();
    if let Ok(extra) = std::env::var("SPIKE_EXTRA") {
        for item in extra.split(';').filter(|s| !s.is_empty()) {
            let (path, pw) = item.split_once("::").unwrap_or((item, ""));
            let label = Path::new(path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            list.push((
                label,
                PathBuf::from(path),
                (!pw.is_empty()).then(|| pw.to_string()),
            ));
        }
    }
    list
}

// ---------------------------------------------------------------------------
// Ham PDFium (form_fill_spike ile aynı FILEWRITE kalıbı)
// ---------------------------------------------------------------------------

#[repr(C)]
struct VecWriter {
    base: FPDF_FILEWRITE,
    out: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    // SAFETY: `this`, canlı bir `VecWriter`ın (repr(C)) ilk alanıdır; PDFium `size` baytlık
    // okunabilir bir tampon verir.
    let writer = unsafe { &mut *(this as *mut VecWriter) };
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    writer.out.extend_from_slice(bytes);
    1
}

struct Doc<'a> {
    b: &'a dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
}

impl<'a> Doc<'a> {
    fn open(b: &'a dyn PdfiumLibraryBindings, bytes: &[u8], pw: Option<&str>) -> Option<Self> {
        let doc = b.FPDF_LoadMemDocument(bytes, pw);
        (!doc.is_null()).then_some(Self { b, doc })
    }

    fn pages(&self) -> i32 {
        self.b.FPDF_GetPageCount(self.doc)
    }

    fn page<R>(&self, index: i32, f: impl FnOnce(FPDF_PAGE) -> R) -> R {
        let page = self.b.FPDF_LoadPage(self.doc, index);
        assert!(!page.is_null(), "FPDF_LoadPage {index}");
        let r = f(page);
        self.b.FPDF_ClosePage(page);
        r
    }

    fn size(&self, index: i32) -> (f32, f32) {
        let mut s = FS_SIZEF {
            width: 0.0,
            height: 0.0,
        };
        self.b.FPDF_GetPageSizeByIndexF(self.doc, index, &mut s);
        (s.width.round(), s.height.round())
    }

    fn save(&self, flags: FPDF_DWORD) -> Option<Vec<u8>> {
        let mut w = Box::new(VecWriter {
            base: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            out: Vec::new(),
        });
        let ok = self.b.FPDF_SaveAsCopy(self.doc, &mut w.base, flags);
        self.b.is_true(ok).then(|| std::mem::take(&mut w.out))
    }
}

impl Drop for Doc<'_> {
    fn drop(&mut self) {
        self.b.FPDF_CloseDocument(self.doc);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn annot_contents(b: &dyn PdfiumLibraryBindings, annot: FPDF_ANNOTATION) -> String {
    let len = b.FPDFAnnot_GetStringValue(annot, "Contents", std::ptr::null_mut(), 0);
    if len <= 2 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) / 2];
    b.FPDFAnnot_GetStringValue(annot, "Contents", buf.as_mut_ptr(), len);
    String::from_utf16_lossy(&buf)
        .trim_end_matches('\0')
        .to_string()
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Rotate,
    DeleteLast,
    InsertBlank,
    AddAnnot,
    RemoveAnnot,
}

/// Mutasyonu uygular; uygulanamazsa (ör. tek sayfalı belgede silme) `None`.
fn apply(d: &Doc<'_>, op: Op) -> Option<()> {
    let b = d.b;
    match op {
        Op::Rotate => d.page(0, |p| {
            let r = b.FPDFPage_GetRotation(p);
            b.FPDFPage_SetRotation(p, (r + 1) % 4);
        }),
        Op::DeleteLast => {
            if d.pages() < 2 {
                return None;
            }
            b.FPDFPage_Delete(d.doc, d.pages() - 1);
        }
        Op::InsertBlank => {
            let page = b.FPDFPage_New(d.doc, 0, 300.0, 200.0);
            assert!(!page.is_null());
            b.FPDF_ClosePage(page);
        }
        Op::AddAnnot => d.page(0, |p| {
            let a = b.FPDFPage_CreateAnnot(p, ANNOT_TEXT);
            assert!(!a.is_null());
            let rect = FS_RECTF {
                left: 20.0,
                top: 60.0,
                right: 60.0,
                bottom: 20.0,
            };
            b.FPDFAnnot_SetRect(a, &rect);
            b.FPDFAnnot_SetColor(a, COLORTYPE_COLOR, 255, 204, 0, 255);
            let w = wide(MARK);
            b.FPDFAnnot_SetStringValue(a, "Contents", w.as_ptr());
            b.FPDFPage_CloseAnnot(a);
        }),
        Op::RemoveAnnot => {
            let count = d.page(0, |p| b.FPDFPage_GetAnnotCount(p));
            if count == 0 {
                return None;
            }
            d.page(0, |p| b.FPDFPage_RemoveAnnot(p, count - 1));
        }
    }
    Some(())
}

/// Mutasyon sonrası beklenen durumun yeniden açılan belgede görülüp görülmediği.
fn persisted(before: &Doc<'_>, after: &Doc<'_>, op: Op) -> bool {
    let b = before.b;
    match op {
        Op::Rotate => {
            let r0 = before.page(0, |p| b.FPDFPage_GetRotation(p));
            after.page(0, |p| after.b.FPDFPage_GetRotation(p)) == (r0 + 1) % 4
        }
        Op::DeleteLast => after.pages() == before.pages() - 1,
        Op::InsertBlank => after.pages() == before.pages() + 1 && after.size(0) == (300.0, 200.0),
        Op::AddAnnot => {
            let n0 = before.page(0, |p| b.FPDFPage_GetAnnotCount(p));
            after.page(0, |p| {
                let n = after.b.FPDFPage_GetAnnotCount(p);
                if n != n0 + 1 {
                    return false;
                }
                let a = after.b.FPDFPage_GetAnnot(p, n - 1);
                let ok = annot_contents(after.b, a) == MARK;
                after.b.FPDFPage_CloseAnnot(a);
                ok
            })
        }
        Op::RemoveAnnot => {
            let n0 = before.page(0, |p| b.FPDFPage_GetAnnotCount(p));
            after.page(0, |p| after.b.FPDFPage_GetAnnotCount(p)) == n0 - 1
        }
    }
}

/// §4.3 maddeleri 1–3 ve PDFium xref stream kusuru.
struct AppendFacts {
    prefix_ok: bool,
    startxref_beyond: bool,
    prev_matches: bool,
    xref_kind: &'static str,
    xrefstream_missing_type: bool,
    xrefstream_missing_endobj: bool,
}

fn append_facts(input: &[u8], output: &[u8]) -> AppendFacts {
    let prefix_ok = output.len() > input.len() && output[..input.len()] == *input;
    let (in_sx, in_off) = last_startxref(input).unwrap_or((0, 0));
    let _ = in_sx;
    let (out_sx, out_off) = last_startxref(output).unwrap_or((0, 0));
    let startxref_beyond = out_off as usize >= input.len();
    let section = if startxref_beyond && (out_off as usize) < out_sx {
        &output[out_off as usize..out_sx]
    } else {
        &[][..]
    };
    let prev_matches = find_first(section, b"/Prev")
        .and_then(|at| number_after(section, at + 5))
        .is_some_and(|prev| prev == in_off);
    let is_table = section.starts_with(b"xref");
    let (mut missing_type, mut missing_endobj) = (false, false);
    if !is_table && !section.is_empty() {
        let dict_end = find_first(section, b"stream").unwrap_or(section.len());
        missing_type = find_first(&section[..dict_end], b"/Type").is_none();
        missing_endobj = find_first(section, b"endobj").is_none();
    }
    AppendFacts {
        prefix_ok,
        startxref_beyond,
        prev_matches,
        xref_kind: if section.is_empty() {
            "-"
        } else if is_table {
            "table"
        } else {
            "stream"
        },
        xrefstream_missing_type: missing_type,
        xrefstream_missing_endobj: missing_endobj,
    }
}

/// `forms::fill::repair_incremental_xref_stream`'in test kopyası (production fonksiyonu private).
fn repair_xref_stream(output: &mut Vec<u8>, original_len: usize) -> bool {
    let Some((startxref, xref_offset)) = last_startxref(output) else {
        return false;
    };
    let xref_offset = xref_offset as usize;
    if xref_offset < original_len || xref_offset >= startxref {
        return false;
    }
    let object = &output[xref_offset..startxref];
    let header_len = object.iter().take_while(|b| b.is_ascii_digit()).count();
    if header_len == 0 || !object[header_len..].starts_with(b" 0 obj <<") {
        return false;
    }
    let dict_start = xref_offset + header_len + b" 0 obj <<".len();
    let Some(stream_kw) = find_first(&output[dict_start..startxref], b">>stream") else {
        return false;
    };
    let dict = &output[dict_start..dict_start + stream_kw];
    if find_first(dict, b"/W[").is_none() || find_first(dict, b"/Index[").is_none() {
        return false;
    }
    let has_type = find_first(dict, b"/Type").is_some();
    let Some(endstream) = find_last(&output[dict_start..startxref], b"endstream") else {
        return false;
    };
    let after = dict_start + endstream + b"endstream".len();
    let mut repaired = false;
    if find_first(&output[after..startxref], b"endobj").is_none() {
        output.splice(after..after, b"\r\nendobj".iter().copied());
        repaired = true;
    }
    if !has_type {
        output.splice(dict_start..dict_start, b"/Type/XRef".iter().copied());
        repaired = true;
    }
    repaired
}

// ---------------------------------------------------------------------------
// S4 aday dedektörü: xref zinciri dosyayla tutarlı mı (onarımsız açılabilir mi)
// ---------------------------------------------------------------------------

fn xref_chain_consistent(bytes: &[u8]) -> Result<(), String> {
    let doc = Document::load_mem(bytes).map_err(|e| format!("lopdf açamadı: {e}"))?;
    let (_, sx) = last_startxref(bytes).ok_or("startxref yok")?;
    let at = bytes.get(sx as usize..).ok_or("startxref dosya dışında")?;
    if !(at.starts_with(b"xref") || at.first().is_some_and(u8::is_ascii_digit)) {
        return Err(format!("startxref {sx} xref başlangıcını göstermiyor"));
    }
    for (num, entry) in &doc.reference_table.entries {
        if let XrefEntry::Normal { offset, generation } = entry {
            let header = format!("{num} {generation} obj");
            let start = *offset as usize;
            let window = bytes
                .get(start..(start + header.len() + 2).min(bytes.len()))
                .unwrap_or(&[]);
            if !window.starts_with(header.as_bytes()) {
                return Err(format!(
                    "nesne {num} offset {offset} '{header}' göstermiyor"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// S1 + S2
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn s1_s2_pdfium_incremental_mutations() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("s1");
    std::fs::create_dir_all(&out).unwrap();
    let mut report = String::from(
        "case\top\tsaved\tprefix\tstartxref>len\tprev\txref\tno_type\tno_endobj\treopen\tpersisted\tsig_before\tsig_after\tplain_mark_in_tail\tgrowth\n",
    );
    for (label, path, pw) in inputs() {
        let input = std::fs::read(&path).unwrap();
        for op in [
            Op::Rotate,
            Op::DeleteLast,
            Op::InsertBlank,
            Op::AddAnnot,
            Op::RemoveAnnot,
        ] {
            let Some(before) = Doc::open(b, &input, pw.as_deref()) else {
                let _ = writeln!(report, "{label}\t{op:?}\tOPEN_FAILED");
                continue;
            };
            let sig_before = b.FPDF_GetSignatureCount(before.doc);
            let work = Doc::open(b, &input, pw.as_deref()).unwrap();
            if apply(&work, op).is_none() {
                continue;
            }
            let Some(mut output) = work.save(FPDF_INCREMENTAL) else {
                let _ = writeln!(
                    report,
                    "{label}\t{op:?}\tSAVE_FAILED(err={})",
                    b.FPDF_GetLastError()
                );
                continue;
            };
            let facts = append_facts(&input, &output);
            let tail = &output[input.len().min(output.len())..];
            let plain_mark = find_first(tail, MARK.as_bytes()).is_some();
            let name = format!("{label}.{op:?}");
            std::fs::write(out.join(format!("{name}.raw.pdf")), &output).unwrap();
            if repair_xref_stream(&mut output, input.len()) {
                std::fs::write(out.join(format!("{name}.repaired.pdf")), &output).unwrap();
            }
            let (reopen, persisted_ok, sig_after) = match Doc::open(b, &output, pw.as_deref()) {
                Some(after) => (
                    true,
                    persisted(&before, &after, op),
                    b.FPDF_GetSignatureCount(after.doc),
                ),
                None => (false, false, -1),
            };
            let _ = writeln!(
                report,
                "{label}\t{op:?}\tok\t{}\t{}\t{}\t{}\t{}\t{}\t{reopen}\t{persisted_ok}\t{sig_before}\t{sig_after}\t{plain_mark}\t{}",
                facts.prefix_ok,
                facts.startxref_beyond,
                facts.prev_matches,
                facts.xref_kind,
                facts.xrefstream_missing_type,
                facts.xrefstream_missing_endobj,
                output.len() as i64 - input.len() as i64,
            );
        }
        // S2 ek: şifreli belgeyi parolasız açmak hâlâ başarısız olmalı.
        if pw.is_some() {
            for file in std::fs::read_dir(&out).unwrap().flatten() {
                let p = file.path();
                if p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("{label}."))
                {
                    let bytes = std::fs::read(&p).unwrap();
                    let opens_without_pw = Doc::open(b, &bytes, None).is_some();
                    let _ = writeln!(
                        report,
                        "#S2 {}\topens_without_password={opens_without_pw}",
                        p.file_name().unwrap().to_string_lossy()
                    );
                }
            }
        }
    }
    std::fs::write(out.join("report.tsv"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// S3: lopdf IncrementalDocument ile sayfa sırası
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn s3_lopdf_incremental_reorder() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("s3");
    std::fs::create_dir_all(&out).unwrap();
    let mut report = String::from(
        "case\tresult\tprefix\tprev\txref\tmid_header\treopen\torder_ok\tsig_after\ttrailer_keys\n",
    );
    for (label, path, pw) in inputs() {
        let input = std::fs::read(&path).unwrap();
        let previous = match Document::load_mem(&input) {
            Ok(d) => d,
            Err(e) => {
                let _ = writeln!(report, "{label}\tLOAD_FAILED({e})");
                continue;
            }
        };
        if previous.is_encrypted() {
            let _ = writeln!(
                report,
                "{label}\tENCRYPTED_SKIPPED(lopdf 0.34 yeni nesneleri şifreleyemez)"
            );
            continue;
        }
        let root = previous
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .unwrap();
        let pages_id = previous
            .get_dictionary(root)
            .unwrap()
            .get(b"Pages")
            .and_then(Object::as_reference)
            .unwrap();
        let kids: Vec<Object> = previous
            .get_dictionary(pages_id)
            .unwrap()
            .get(b"Kids")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        let flat = kids.iter().all(|k| {
            k.as_reference()
                .ok()
                .and_then(|id| previous.get_dictionary(id).ok())
                .and_then(|d| {
                    d.get(b"Type")
                        .ok()
                        .and_then(|t| t.as_name().ok())
                        .map(|n| n == b"Page")
                })
                .unwrap_or(false)
        });
        if !flat {
            let _ = writeln!(report, "{label}\tNESTED_TREE_SKIPPED");
            continue;
        }
        let before = Doc::open(b, &input, pw.as_deref()).unwrap();
        let sizes: Vec<(f32, f32)> = (0..before.pages()).map(|i| before.size(i)).collect();
        let mut incremental = IncrementalDocument::create_from(input.clone(), previous);
        incremental
            .opt_clone_object_to_new_document(pages_id)
            .unwrap();
        let mut reversed = kids.clone();
        reversed.reverse();
        incremental
            .new_document
            .get_object_mut(pages_id)
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Kids", Object::Array(reversed));
        let mut output = Vec::new();
        if let Err(e) = incremental.save_to(&mut output) {
            let _ = writeln!(report, "{label}\tSAVE_FAILED({e})");
            continue;
        }
        let facts = append_facts(&input, &output);
        let tail = &output[input.len()..];
        let mid_header = find_first(tail, b"%PDF-").is_some();
        let (sx, off) = last_startxref(&output).unwrap();
        let section = &output[(off as usize).min(sx)..sx];
        let dict_end = find_first(section, b"stream").unwrap_or(section.len());
        let keys: Vec<String> = String::from_utf8_lossy(&section[..dict_end])
            .split('/')
            .skip(1)
            .map(|k| {
                k.split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|k| !k.is_empty() && k.chars().next().unwrap().is_ascii_uppercase())
            .collect();
        std::fs::write(out.join(format!("{label}.reorder.pdf")), &output).unwrap();
        let (reopen, order_ok, sig_after) = match Doc::open(b, &output, pw.as_deref()) {
            Some(after) => {
                let got: Vec<(f32, f32)> = (0..after.pages()).map(|i| after.size(i)).collect();
                let mut want = sizes.clone();
                want.reverse();
                (true, got == want, b.FPDF_GetSignatureCount(after.doc))
            }
            None => (false, false, -1),
        };
        let _ = writeln!(
            report,
            "{label}\tok\t{}\t{}\t{}\t{mid_header}\t{reopen}\t{order_ok}\t{sig_after}\t{}",
            facts.prefix_ok,
            facts.prev_matches,
            facts.xref_kind,
            keys.join(","),
        );
    }
    std::fs::write(out.join("report.tsv"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// S4: xref onarımı tespiti
// ---------------------------------------------------------------------------

fn corrupt_variants(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let minimal = std::fs::read(fixtures().join("pdf/three-distinct-pages.pdf")).unwrap();
    let mut v = vec![(
        "malformed".to_string(),
        std::fs::read(fixtures().join("pdf/malformed.pdf")).unwrap(),
    )];
    // startxref 7 bayt kaydırılmış.
    let (sx, off) = last_startxref(&minimal).unwrap();
    let mut shifted = minimal[..sx].to_vec();
    shifted.extend_from_slice(format!("startxref\n{}\n%%EOF\n", off + 7).as_bytes());
    v.push(("startxref-shifted".into(), shifted));
    // xref tablosundaki ilk kullanılan girdinin offset'i bozulmuş (uzunluk aynı).
    let mut entry = minimal.clone();
    let xref = off as usize;
    let line = find_first(&entry[xref..], b" 00000 n")
        .map(|p| xref + p - 10)
        .unwrap();
    entry[line..line + 10].copy_from_slice(b"0000000003");
    v.push(("entry-offset-wrong".into(), entry));
    // Başına 3 bayt çöp eklenmiş: tüm offset'ler kayar.
    let mut prefixed = b"XYZ".to_vec();
    prefixed.extend_from_slice(&minimal);
    v.push(("leading-garbage".into(), prefixed));
    // Sondaki startxref/EOF kesilmiş.
    v.push(("truncated-tail".into(), minimal[..sx].to_vec()));
    for (name, bytes) in &v {
        std::fs::write(dir.join(format!("{name}.input.pdf")), bytes).unwrap();
    }
    v
}

#[test]
#[ignore]
fn s4_xref_repair_detection() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("s4");
    std::fs::create_dir_all(&out).unwrap();
    let mut report = String::from("case\tkind\tpdfium_opens\tdetector\tinc_saved\tinc_prefix\tinc_chain_consistent\tinc_reopen\n");
    let mut cases: Vec<(String, &str, Vec<u8>)> = inputs()
        .into_iter()
        .filter(|(_, _, pw)| pw.is_none())
        .map(|(l, p, _)| (l, "good", std::fs::read(p).unwrap()))
        .collect();
    cases.extend(
        corrupt_variants(&out)
            .into_iter()
            .map(|(l, bytes)| (l, "corrupt", bytes)),
    );
    for (label, kind, input) in cases {
        let detector = match xref_chain_consistent(&input) {
            Ok(()) => "consistent".to_string(),
            Err(e) => format!("INCONSISTENT: {e}"),
        };
        let Some(doc) = Doc::open(b, &input, None) else {
            let _ = writeln!(report, "{label}\t{kind}\tfalse\t{detector}");
            continue;
        };
        apply(&doc, Op::Rotate);
        let (saved, prefix, chain, reopen) = match doc.save(FPDF_INCREMENTAL) {
            Some(mut output) => {
                repair_xref_stream(&mut output, input.len());
                std::fs::write(out.join(format!("{label}.rotate.pdf")), &output).unwrap();
                let chain = match xref_chain_consistent(&output) {
                    Ok(()) => "consistent".to_string(),
                    Err(e) => format!("INCONSISTENT: {e}"),
                };
                (
                    true,
                    output.starts_with(&input),
                    chain,
                    Doc::open(b, &output, None).is_some(),
                )
            }
            None => (false, false, "-".into(), false),
        };
        let _ = writeln!(
            report,
            "{label}\t{kind}\ttrue\t{detector}\t{saved}\t{prefix}\t{chain}\t{reopen}"
        );
    }
    std::fs::write(out.join("report.tsv"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// S5: geçmiş için hard link + atomik rename (yalnız dosya sistemi semantiği)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
#[ignore]
fn s5_hardlink_retention_semantics() {
    use std::os::unix::fs::MetadataExt;
    let dir = out_dir().join("s5");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("history")).unwrap();
    let working = dir.join("working.pdf");
    let temp = dir.join(".tmp-new.pdf");
    let retained = dir.join("history/gen-1.pdf");
    std::fs::write(&working, b"old-content").unwrap();
    std::fs::write(&temp, b"new-content").unwrap();
    let old_inode = std::fs::metadata(&working).unwrap().ino();
    let mut r = String::new();
    let _ = writeln!(
        r,
        "before: working nlink={}",
        std::fs::metadata(&working).unwrap().nlink()
    );
    // Kilit altındaki link kontrolü burada yapılmış sayılır (nlink == 1); sonra link + rename.
    std::fs::hard_link(&working, &retained).unwrap();
    let _ = writeln!(
        r,
        "after link: working nlink={} (bu anda bir kontrol çalışsa reddederdi)",
        std::fs::metadata(&working).unwrap().nlink()
    );
    std::fs::rename(&temp, &working).unwrap();
    let w = std::fs::metadata(&working).unwrap();
    let h = std::fs::metadata(&retained).unwrap();
    let _ = writeln!(
        r,
        "after rename: working nlink={} new_inode={}",
        w.nlink(),
        w.ino() != old_inode
    );
    let _ = writeln!(
        r,
        "retained nlink={} same_old_inode={} content={:?}",
        h.nlink(),
        h.ino() == old_inode,
        std::fs::read_to_string(&retained).unwrap()
    );
    // (c) Redo gerekmiyorsa blob rename ile working'in yerine geçebilir mi?
    std::fs::rename(&retained, &working).unwrap();
    let w2 = std::fs::metadata(&working).unwrap();
    let _ = writeln!(
        r,
        "restore by rename: working nlink={} content={:?} history_empty={}",
        w2.nlink(),
        std::fs::read_to_string(&working).unwrap(),
        std::fs::read_dir(dir.join("history")).unwrap().count() == 0
    );
    // Rename başarısız olursa (ör. hedef dizin yok) link geride kalır mı?
    std::fs::write(&temp, b"new2").unwrap();
    let orphan = dir.join("history/gen-2.pdf");
    std::fs::hard_link(&working, &orphan).unwrap();
    let failed = std::fs::rename(&temp, dir.join("missing-dir/working.pdf")).is_err();
    let _ = writeln!(
        r,
        "rename failure: failed={failed} working nlink={} -> link geri alınmalı",
        std::fs::metadata(&working).unwrap().nlink()
    );
    std::fs::write(dir.join("report.txt"), &r).unwrap();
    println!("{r}");
}

// ---------------------------------------------------------------------------
// S6: maliyet ölçümü
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn s6_cost_measurement() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("s6");
    std::fs::create_dir_all(&out).unwrap();
    let paths: Vec<PathBuf> = std::env::var("SPIKE_PERF")
        .unwrap_or_default()
        .split(';')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let mut report = String::from("file\tbytes\tpages\tload_ms\tinc_save_ms\tinc_growth\tfull_save_ms\tfull_bytes\tprefix_copy_hash_ms\tsha_only_ms\n");
    for path in paths {
        let input = std::fs::read(&path).unwrap();
        let t = Instant::now();
        let doc = Doc::open(b, &input, None).unwrap();
        let load_ms = t.elapsed().as_millis();
        apply(&doc, Op::Rotate).unwrap();
        let t = Instant::now();
        let inc = doc.save(FPDF_INCREMENTAL).unwrap();
        let inc_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let full = doc.save(FPDF_FULL_REWRITE).unwrap();
        let full_ms = t.elapsed().as_millis();
        // Undo maliyeti: güncel dosyanın prefix'ini temp'e akıt + fsync + hash.
        let src = out.join("current.pdf");
        std::fs::write(&src, &inc).unwrap();
        let t = Instant::now();
        {
            use std::io::{Read, Write};
            let mut reader = std::fs::File::open(&src).unwrap().take(input.len() as u64);
            let mut file = std::fs::File::create(out.join("restored.pdf")).unwrap();
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = reader.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                file.write_all(&buf[..n]).unwrap();
            }
            file.sync_all().unwrap();
            let _ = hasher.finalize();
        }
        let restore_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let _ = sha(&input);
        let sha_ms = t.elapsed().as_millis();
        let _ = writeln!(
            report,
            "{}\t{}\t{}\t{load_ms}\t{inc_ms}\t{}\t{full_ms}\t{}\t{restore_ms}\t{sha_ms}",
            path.file_name().unwrap().to_string_lossy(),
            input.len(),
            doc.pages(),
            inc.len() - input.len(),
            full.len(),
        );
    }
    std::fs::write(out.join("report.tsv"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// S3b: iç içe sayfa ağacı + miras alınan özellikler, düzleştirerek sıralama
// ---------------------------------------------------------------------------

/// Kök (MediaBox 600x800, Rotate 90, Resources/F1) → [Ara düğüm A (MediaBox 300x400) →
/// [P1, P2 (Rotate 0)], P3 (MediaBox 500x500)]. Sayfa içerikleri yalnız mirasla gelen F1'i kullanır.
fn nested_tree_fixture() -> Vec<u8> {
    use lopdf::{dictionary, Stream};
    let mut doc = Document::with_version("1.7");
    let root_id = doc.new_object_id();
    let a_id = doc.new_object_id();
    let font = doc.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let page = |doc: &mut Document, parent, label: &str, extra: lopdf::Dictionary| {
        let content = format!("BT /F1 24 Tf 40 40 Td ({label}) Tj ET");
        let c = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let mut d = dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => c };
        for (k, v) in extra.iter() {
            d.set(k.clone(), v.clone());
        }
        doc.add_object(d)
    };
    let p1 = page(
        &mut doc,
        Object::Reference(a_id),
        "NESTED-P1",
        dictionary! {},
    );
    let p2 = page(
        &mut doc,
        Object::Reference(a_id),
        "NESTED-P2",
        dictionary! { "Rotate" => 0 },
    );
    let p3 = page(
        &mut doc,
        Object::Reference(root_id),
        "NESTED-P3",
        dictionary! { "MediaBox" => vec![0.into(), 0.into(), 500.into(), 500.into()] },
    );
    doc.objects.insert(
        a_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Parent" => root_id, "Count" => 2,
            "Kids" => vec![p1.into(), p2.into()],
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
        }),
    );
    doc.objects.insert(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Count" => 3, "Kids" => vec![a_id.into(), p3.into()],
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Rotate" => 90,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn page_text(d: &Doc<'_>, index: i32) -> String {
    let b = d.b;
    d.page(index, |p| {
        let t = b.FPDFText_LoadPage(p);
        let n = b.FPDFText_CountChars(t);
        let mut buf = vec![0u16; n as usize + 1];
        b.FPDFText_GetText(t, 0, n, buf.as_mut_ptr());
        b.FPDFText_ClosePage(t);
        String::from_utf16_lossy(&buf)
            .trim_end_matches('\0')
            .trim()
            .to_string()
    })
}

fn page_facts(d: &Doc<'_>) -> Vec<(String, (f32, f32), i32)> {
    (0..d.pages())
        .map(|i| {
            (
                page_text(d, i),
                d.size(i),
                d.page(i, |p| d.b.FPDFPage_GetRotation(p)),
            )
        })
        .collect()
}

#[test]
#[ignore]
fn s3b_lopdf_flatten_nested_tree_reorder() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("s3b");
    std::fs::create_dir_all(&out).unwrap();
    let input = nested_tree_fixture();
    std::fs::write(out.join("nested.input.pdf"), &input).unwrap();
    let before = Doc::open(b, &input, None).unwrap();
    let facts_before = page_facts(&before);

    let previous = Document::load_mem(&input).unwrap();
    let root = previous
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .unwrap();
    let pages_root = previous
        .get_dictionary(root)
        .unwrap()
        .get(b"Pages")
        .and_then(Object::as_reference)
        .unwrap();
    // Belge sırasıyla yaprak sayfalar ve her birinin etkin miras değerleri.
    const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
    let mut leaves = Vec::new();
    fn walk(
        doc: &Document,
        node: lopdf::ObjectId,
        inherited: &lopdf::Dictionary,
        leaves: &mut Vec<(lopdf::ObjectId, lopdf::Dictionary)>,
    ) {
        let dict = doc.get_dictionary(node).unwrap();
        let mut here = inherited.clone();
        for key in INHERITABLE {
            if let Ok(v) = dict.get(key) {
                here.set(key.to_vec(), v.clone());
            }
        }
        let kind = dict
            .get(b"Type")
            .and_then(Object::as_name)
            .unwrap_or_else(|_| panic!("node {node:?} Type yok: {dict:?}"));
        if kind == b"Page" {
            leaves.push((node, here));
        } else {
            for kid in dict.get(b"Kids").unwrap().as_array().unwrap() {
                walk(doc, kid.as_reference().unwrap(), &here, leaves);
            }
        }
    }
    walk(
        &previous,
        pages_root,
        &lopdf::Dictionary::new(),
        &mut leaves,
    );

    let mut inc = IncrementalDocument::create_from(input.clone(), previous);
    let mut order: Vec<Object> = Vec::new();
    for (id, effective) in leaves.iter().rev() {
        inc.opt_clone_object_to_new_document(*id).unwrap();
        let page = inc
            .new_document
            .get_object_mut(*id)
            .and_then(Object::as_dict_mut)
            .unwrap();
        for (key, value) in effective.iter() {
            if page.get(key).is_err() {
                page.set(key.clone(), value.clone());
            }
        }
        page.set("Parent", Object::Reference(pages_root));
        order.push(Object::Reference(*id));
    }
    inc.opt_clone_object_to_new_document(pages_root).unwrap();
    let root_dict = inc
        .new_document
        .get_object_mut(pages_root)
        .and_then(Object::as_dict_mut)
        .unwrap();
    root_dict.set("Kids", Object::Array(order.clone()));
    root_dict.set("Count", Object::Integer(order.len() as i64));
    // Miras artık sayfalarda; kökte bırakmak zararsız ama tek kaynak olsun diye kaldırılır.
    for key in INHERITABLE {
        root_dict.remove(key);
    }
    let mut output = Vec::new();
    inc.save_to(&mut output).unwrap();
    std::fs::write(out.join("nested.reorder.pdf"), &output).unwrap();

    let facts = append_facts(&input, &output);
    let after = Doc::open(b, &output, None).expect("PDFium reopen");
    let facts_after = page_facts(&after);
    let mut want = facts_before.clone();
    want.reverse();
    let chain = xref_chain_consistent(&output);
    let report = format!(
        "before={facts_before:?}\nafter ={facts_after:?}\nwant  ={want:?}\norder_ok={} prefix={} prev={} chain={chain:?}\n",
        facts_after == want,
        facts.prefix_ok,
        facts.prev_matches,
    );
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// S6b: aynı döndürmenin lopdf ile artımlı büyümesi (yalnız bayt; süre ölçülmez)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn s6b_lopdf_rotate_growth() {
    let paths: Vec<PathBuf> = std::env::var("SPIKE_PERF")
        .unwrap_or_default()
        .split(';')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let mut report = String::from("file\tbytes\tlopdf_rotate_growth\n");
    for path in paths {
        let input = std::fs::read(&path).unwrap();
        let previous = Document::load_mem(&input).unwrap();
        let first = *previous.get_pages().get(&1).unwrap();
        let mut inc = IncrementalDocument::create_from(input.clone(), previous);
        inc.opt_clone_object_to_new_document(first).unwrap();
        inc.new_document
            .get_object_mut(first)
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Rotate", Object::Integer(90));
        let mut output = Vec::new();
        inc.save_to(&mut output).unwrap();
        assert!(output.starts_with(&input));
        let _ = writeln!(
            report,
            "{}\t{}\t{}",
            path.file_name().unwrap().to_string_lossy(),
            input.len(),
            output.len() - input.len()
        );
    }
    println!("{report}");
}

// ---------------------------------------------------------------------------
// K5 deneyi: PDFium düzenlesin (AP üretimi dahil), lopdf yalnız değişen nesneleri yazsın
// ---------------------------------------------------------------------------

fn render_hash(d: &Doc<'_>, index: i32) -> String {
    let b = d.b;
    d.page(index, |p| {
        let (w, h) = (400, 566);
        let bmp = b.FPDFBitmap_Create(w, h, 0);
        b.FPDFBitmap_FillRect(bmp, 0, 0, w, h, 0xFFFF_FFFF);
        b.FPDF_RenderPageBitmap(bmp, p, 0, 0, w, h, 0, 1 /* FPDF_ANNOT */);
        let stride = b.FPDFBitmap_GetStride(bmp) as usize;
        // SAFETY: PDFium'un ayırdığı `stride * h` baytlık tampon, bitmap yok edilmeden okunur.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                b.FPDFBitmap_GetBuffer(bmp) as *const u8,
                stride * h as usize,
            )
        }
        .to_vec();
        b.FPDFBitmap_Destroy(bmp);
        sha(&bytes)
    })
}

fn same_object(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Stream(x), Object::Stream(y)) => x.dict == y.dict && x.content == y.content,
        _ => a == b,
    }
}

/// PDFium çıktısındaki her nesneyi girdideki son sürümüyle anlamsal olarak karşılaştırır;
/// yalnız yeni ve değişen nesneleri girdinin üzerine lopdf artımlı güncellemesi olarak yazar.
fn minimize_with_lopdf(input: &[u8], pdfium_out: &[u8]) -> (Vec<u8>, usize, usize) {
    let prev = Document::load_mem(input).unwrap();
    let next = Document::load_mem(pdfium_out).unwrap();
    let mut changed = Vec::new();
    for (id, obj) in &next.objects {
        let same = prev
            .objects
            .get(id)
            .is_some_and(|old| same_object(old, obj));
        if !same && !matches!(obj.type_name().ok(), Some(b"XRef" | b"ObjStm")) {
            changed.push(*id);
        }
    }
    let total = next.objects.len();
    let mut inc = IncrementalDocument::create_from(input.to_vec(), prev);
    for id in &changed {
        inc.new_document
            .objects
            .insert(*id, next.objects[id].clone());
        inc.new_document.max_id = inc.new_document.max_id.max(id.0);
    }
    if let Ok(root) = next.trailer.get(b"Root") {
        inc.new_document.trailer.set("Root", root.clone());
    }
    inc.new_document
        .trailer
        .set("Size", Object::Integer(inc.new_document.max_id as i64 + 1));
    let mut out = Vec::new();
    inc.save_to(&mut out).unwrap();
    (out, changed.len(), total)
}

#[test]
#[ignore]
fn k5_pdfium_edit_lopdf_minimal_write() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let out = out_dir().join("k5");
    std::fs::create_dir_all(&out).unwrap();
    let path = PathBuf::from(std::env::var("SPIKE_K5").expect("SPIKE_K5: görselli girdi"));
    let input = std::fs::read(&path).unwrap();
    let mut report = String::from("op\tpdfium_growth\tminimal_growth\tchanged/total_objects\tprefix\tprev\tchain\treopen\trender_equal\tannots\n");
    for op in ["highlight", "rotate"] {
        let work = Doc::open(b, &input, None).unwrap();
        if op == "highlight" {
            work.page(0, |p| {
                let a = b.FPDFPage_CreateAnnot(p, 9 /* FPDF_ANNOT_HIGHLIGHT */);
                let rect = FS_RECTF {
                    left: 60.0,
                    top: 760.0,
                    right: 300.0,
                    bottom: 740.0,
                };
                b.FPDFAnnot_SetRect(a, &rect);
                b.FPDFAnnot_SetColor(a, COLORTYPE_COLOR, 255, 204, 0, 255);
                let q = FS_QUADPOINTSF {
                    x1: 60.0,
                    y1: 760.0,
                    x2: 300.0,
                    y2: 760.0,
                    x3: 60.0,
                    y3: 740.0,
                    x4: 300.0,
                    y4: 740.0,
                };
                b.FPDFAnnot_AppendAttachmentPoints(a, &q);
                b.FPDFPage_CloseAnnot(a);
            });
            // Production gibi: AP, sayfa annotation'larla render edilirken üretilir (annotations.rs:352).
            let _ = render_hash(&work, 0);
        } else {
            apply(&work, Op::Rotate).unwrap();
        }
        let mut raw = work.save(FPDF_INCREMENTAL).unwrap();
        repair_xref_stream(&mut raw, input.len());
        std::fs::write(out.join(format!("{op}.pdfium.pdf")), &raw).unwrap();
        let reference = Doc::open(b, &raw, None).unwrap();
        let want = render_hash(&reference, 0);
        let (min, changed, total) = minimize_with_lopdf(&input, &raw);
        std::fs::write(out.join(format!("{op}.minimal.pdf")), &min).unwrap();
        let facts = append_facts(&input, &min);
        let chain = xref_chain_consistent(&min).is_ok();
        let (reopen, equal, annots) = match Doc::open(b, &min, None) {
            Some(d) => (
                true,
                render_hash(&d, 0) == want,
                d.page(0, |p| b.FPDFPage_GetAnnotCount(p)),
            ),
            None => (false, false, -1),
        };
        let _ = writeln!(
            report,
            "{op}\t{}\t{}\t{changed}/{total}\t{}\t{}\t{chain}\t{reopen}\t{equal}\t{annots}",
            raw.len() - input.len(),
            min.len() - input.len(),
            facts.prefix_ok,
            facts.prev_matches,
        );
    }
    std::fs::write(out.join("report.tsv"), &report).unwrap();
    println!("{report}");
}

// ---------------------------------------------------------------------------
// K5-B maliyeti: büyük belgede lopdf yükleme + fark + yazım + render doğrulaması
// Her dosya ayrı süreçte çalıştırılır (SPIKE_ONE) ki tepe bellek `/usr/bin/time -l` ile ölçülsün.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn k5b_minimize_cost() {
    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();
    let path = PathBuf::from(std::env::var("SPIKE_ONE").expect("SPIKE_ONE"));
    let mode = std::env::var("SPIKE_MODE").unwrap_or_else(|_| "full".into());
    let input = std::fs::read(&path).unwrap();
    let work = Doc::open(b, &input, None).unwrap();
    apply(&work, Op::Rotate).unwrap();
    let mut raw = work.save(FPDF_INCREMENTAL).unwrap();
    repair_xref_stream(&mut raw, input.len());
    drop(work);
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    if mode == "pdfium-only" {
        println!(
            "RESULT\t{name}\t{}\tpdfium-only\tgrowth={}",
            input.len(),
            raw.len() - input.len()
        );
        return;
    }
    let t = Instant::now();
    let prev = Document::load_mem(&input).unwrap();
    let load_prev_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let next = Document::load_mem(&raw).unwrap();
    let load_next_ms = t.elapsed().as_millis();
    let objects = next.objects.len();
    drop((prev, next));
    let t = Instant::now();
    let (min, changed, _) = minimize_with_lopdf(&input, &raw);
    let minimize_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let want = render_hash(&Doc::open(b, &raw, None).unwrap(), 0);
    let got = render_hash(&Doc::open(b, &min, None).unwrap(), 0);
    let verify_ms = t.elapsed().as_millis();
    println!(
        "RESULT\t{name}\t{}\tobjects={objects}\tload_prev_ms={load_prev_ms}\tload_next_ms={load_next_ms}\tminimize_total_ms={minimize_ms}\tverify_render_ms={verify_ms}\tchanged={changed}\tpdfium_growth={}\tminimal_growth={}\trender_equal={}",
        input.len(),
        raw.len() - input.len(),
        min.len() - input.len(),
        want == got,
    );
}
