use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use parking_lot::Mutex;
use pdfium_render::prelude::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const OLD_REVISION_MARKER: &[u8] = b"SANITIZATION_OLD_REVISION_PAYLOAD_001";
const XREF_STREAM_OLD_REVISION_MARKER: &[u8] = b"SANITIZATION_XREF_STREAM_OLD_REVISION_001";
const EMBEDDED_MARKER: &[u8] = b"SANITIZATION_EMBEDDED_PAYLOAD_001";
static PDFIUM_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureManifest {
    schema_version: String,
    description: String,
    fixtures: Vec<FixtureManifestItem>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureManifestItem {
    name: String,
    purpose: String,
    threat_categories: Vec<String>,
    expected_inspection: Vec<String>,
    candidate_controls: Vec<String>,
    blocking_unsupported: Vec<String>,
    expected_visual_impact: String,
    expected_functional_impact: String,
    future_transform_expectation: String,
    future_verification_expectation: String,
    notes: String,
    sha256: String,
    byte_size: usize,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sanitization")
}

/// lopdf 0.45 consumes /Prev (and /XRefStm) while loading; the raw last trailer keeps it.
fn raw_trailer_history(name: &str) -> preview_alternative_lib::lopdf_compat::TrailerHistory {
    let bytes = fs::read(fixture_dir().join(name)).unwrap();
    let document = Document::load_mem(&bytes).unwrap();
    preview_alternative_lib::lopdf_compat::trailer_history(&bytes, &document)
        .unwrap_or_else(|| panic!("{name}: last trailer not found"))
}

fn load_fixture(name: &str) -> Document {
    Document::load(fixture_dir().join(name))
        .unwrap_or_else(|error| panic!("lopdf failed to parse {name}: {error:?}"))
}

fn read_manifest() -> FixtureManifest {
    serde_json::from_slice(
        &fs::read(fixture_dir().join("manifest.json")).expect("read sanitization manifest"),
    )
    .expect("strictly parse sanitization manifest")
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        other => Some(other),
    }
}

fn resolve_dictionary<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve_object(document, object)? {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn parse_startxref(output: &[u8]) -> usize {
    let marker = output
        .windows(b"startxref".len())
        .rposition(|window| window == b"startxref")
        .expect("startxref marker");
    String::from_utf8_lossy(&output[marker + b"startxref".len()..])
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("startxref value")
        .trim()
        .parse()
        .expect("numeric startxref")
}

fn validate_classic_xref_points_to_output_objects(output: &[u8]) -> (usize, usize) {
    let startxref = parse_startxref(output);
    assert!(startxref < output.len());
    assert!(
        output[startxref..].starts_with(b"xref\n"),
        "writer emitted non-classic xref at startxref: {:?}",
        String::from_utf8_lossy(&output[startxref..output.len().min(startxref + 96)])
    );

    let mut lines = output[startxref..].split(|byte| *byte == b'\n');
    assert_eq!(lines.next(), Some(b"xref".as_slice()));
    let mut in_use = 0;
    let mut free = 0;
    loop {
        let line = lines.next().expect("xref subsection or trailer");
        if line == b"trailer" {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let header = String::from_utf8_lossy(line);
        let mut header_parts = header.split_ascii_whitespace();
        let first_object: u32 = header_parts.next().unwrap().parse().unwrap();
        let count: u32 = header_parts.next().unwrap().parse().unwrap();
        assert!(
            header_parts.next().is_none(),
            "invalid xref subsection header"
        );
        for index in 0..count {
            let entry = String::from_utf8_lossy(lines.next().expect("xref entry"));
            let mut parts = entry.split_ascii_whitespace();
            let offset: usize = parts.next().unwrap().parse().unwrap();
            let generation: u16 = parts.next().unwrap().parse().unwrap();
            let kind = parts.next().unwrap();
            assert!(parts.next().is_none(), "invalid xref entry");
            match kind {
                "n" => {
                    assert!(offset < output.len());
                    let expected = format!("{} {} obj", first_object + index, generation);
                    assert!(
                        output[offset..].starts_with(expected.as_bytes()),
                        "xref entry {} {} points to non-object bytes at {offset}",
                        first_object + index,
                        generation
                    );
                    in_use += 1;
                }
                "f" => {
                    free += 1;
                }
                other => panic!("unknown xref entry kind {other}"),
            }
        }
    }
    assert!(in_use > 0);
    assert!(free > 0);
    (in_use, free)
}

fn parse_compact_integer_array(dictionary: &[u8], key: &[u8]) -> Vec<usize> {
    let key_offset = dictionary
        .windows(key.len())
        .position(|window| window == key)
        .expect("xref dictionary key");
    let after_key = &dictionary[key_offset + key.len()..];
    let open = after_key.iter().position(|byte| *byte == b'[').unwrap();
    let close = after_key[open + 1..]
        .iter()
        .position(|byte| *byte == b']')
        .unwrap()
        + open
        + 1;
    String::from_utf8_lossy(&after_key[open + 1..close])
        .split_ascii_whitespace()
        .map(|value| value.parse().unwrap())
        .collect()
}

fn read_big_endian(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .fold(0usize, |value, byte| (value << 8) | usize::from(*byte))
}

fn validate_xref_stream_points_to_output_objects(output: &[u8]) -> usize {
    let startxref = parse_startxref(output);
    assert!(startxref < output.len());
    let tail = &output[startxref..];
    let header_end = tail.iter().position(|byte| *byte == b'\n').unwrap();
    let header = String::from_utf8_lossy(&tail[..header_end]);
    let mut header_parts = header.split_ascii_whitespace();
    let xref_object: u32 = header_parts.next().unwrap().parse().unwrap();
    let generation: u16 = header_parts.next().unwrap().parse().unwrap();
    assert_eq!(header_parts.next(), Some("obj"));
    assert!(header_parts.next().is_none());

    let stream_marker = tail
        .windows(b"stream\n".len())
        .position(|window| window == b"stream\n")
        .expect("xref stream marker");
    let dictionary = &tail[header_end + 1..stream_marker];
    assert!(dictionary
        .windows(b"/Type/XRef".len())
        .any(|window| window == b"/Type/XRef"));
    let widths = parse_compact_integer_array(dictionary, b"/W");
    let indices = parse_compact_integer_array(dictionary, b"/Index");
    assert_eq!(widths.len(), 3);
    assert_eq!(indices.len() % 2, 0);
    let entry_width: usize = widths.iter().sum();
    let entry_count: usize = indices.as_chunks::<2>().0.iter().map(|pair| pair[1]).sum();
    let content_start = stream_marker + b"stream\n".len();
    let content_end = content_start + entry_width * entry_count;
    assert!(tail[content_end..].starts_with(b"\nendstream"));

    let mut cursor = content_start;
    let mut in_use = 0;
    for pair in indices.as_chunks::<2>().0 {
        for object_number in pair[0]..pair[0] + pair[1] {
            let entry_type = if widths[0] == 0 {
                1
            } else {
                read_big_endian(&tail[cursor..cursor + widths[0]])
            };
            cursor += widths[0];
            let field_two = read_big_endian(&tail[cursor..cursor + widths[1]]);
            cursor += widths[1];
            let field_three = read_big_endian(&tail[cursor..cursor + widths[2]]);
            cursor += widths[2];
            match entry_type {
                0 => {}
                1 => {
                    assert!(field_two < output.len());
                    let expected = format!("{object_number} {field_three} obj");
                    assert!(
                        output[field_two..].starts_with(expected.as_bytes()),
                        "xref-stream entry {object_number} {field_three} points outside its output object"
                    );
                    in_use += 1;
                }
                2 => panic!("full rewrite unexpectedly retained a compressed-object xref entry"),
                other => panic!("unknown xref-stream entry type {other}"),
            }
        }
    }
    assert_eq!(cursor, content_end);
    let last_pair = &indices[indices.len() - 2..];
    assert_eq!(xref_object as usize, last_pair[0] + last_pair[1] - 1);
    assert_eq!(generation, 0);
    assert!(in_use > 0);
    in_use
}

fn object_contains_marker(document: &Document, object: &Object, marker: &[u8]) -> bool {
    fn walk(
        document: &Document,
        object: &Object,
        marker: &[u8],
        visited: &mut HashSet<ObjectId>,
    ) -> bool {
        match object {
            Object::Reference(id) => {
                if !visited.insert(*id) {
                    return false;
                }
                document
                    .get_object(*id)
                    .is_ok_and(|resolved| walk(document, resolved, marker, visited))
            }
            Object::String(bytes, _) | Object::Name(bytes) => {
                bytes.windows(marker.len()).any(|window| window == marker)
            }
            Object::Array(array) => array
                .iter()
                .any(|item| walk(document, item, marker, visited)),
            Object::Dictionary(dictionary) => dictionary
                .iter()
                .any(|(_, item)| walk(document, item, marker, visited)),
            Object::Stream(stream) => {
                stream
                    .content
                    .windows(marker.len())
                    .any(|window| window == marker)
                    || stream
                        .decompressed_content()
                        .is_ok_and(|decoded| decoded.windows(marker.len()).any(|w| w == marker))
                    || stream
                        .dict
                        .iter()
                        .any(|(_, item)| walk(document, item, marker, visited))
            }
            _ => false,
        }
    }

    walk(document, object, marker, &mut HashSet::new())
}

fn document_contains_marker(document: &Document, marker: &[u8]) -> bool {
    document
        .objects
        .values()
        .any(|object| object_contains_marker(document, object, marker))
}

#[derive(Default, Debug)]
struct WalkEvidence {
    indirect_objects: usize,
    direct_objects: usize,
    unresolved_references: Vec<ObjectId>,
    depth_limited: bool,
    object_limited: bool,
}

fn bounded_walk(document: &Document, max_depth: usize, max_objects: usize) -> WalkEvidence {
    fn walk(
        document: &Document,
        object: &Object,
        depth: usize,
        max_depth: usize,
        max_objects: usize,
        visited: &mut HashSet<ObjectId>,
        evidence: &mut WalkEvidence,
    ) {
        if evidence.indirect_objects + evidence.direct_objects >= max_objects {
            evidence.object_limited = true;
            return;
        }
        if depth > max_depth {
            evidence.depth_limited = true;
            return;
        }
        match object {
            Object::Reference(id) => {
                if !visited.insert(*id) {
                    return;
                }
                evidence.indirect_objects += 1;
                match document.get_object(*id) {
                    Ok(value) => walk(
                        document,
                        value,
                        depth + 1,
                        max_depth,
                        max_objects,
                        visited,
                        evidence,
                    ),
                    Err(_) => evidence.unresolved_references.push(*id),
                }
            }
            Object::Array(items) => {
                evidence.direct_objects += 1;
                for item in items {
                    walk(
                        document,
                        item,
                        depth + 1,
                        max_depth,
                        max_objects,
                        visited,
                        evidence,
                    );
                }
            }
            Object::Dictionary(dictionary) => {
                evidence.direct_objects += 1;
                for (_, item) in dictionary {
                    walk(
                        document,
                        item,
                        depth + 1,
                        max_depth,
                        max_objects,
                        visited,
                        evidence,
                    );
                }
            }
            Object::Stream(stream) => {
                evidence.direct_objects += 1;
                for (_, item) in &stream.dict {
                    walk(
                        document,
                        item,
                        depth + 1,
                        max_depth,
                        max_objects,
                        visited,
                        evidence,
                    );
                }
            }
            _ => evidence.direct_objects += 1,
        }
    }

    let mut evidence = WalkEvidence::default();
    let mut visited = HashSet::new();
    let trailer = Object::Dictionary(document.trailer.clone());
    walk(
        document,
        &trailer,
        0,
        max_depth,
        max_objects,
        &mut visited,
        &mut evidence,
    );
    evidence
}

fn collect_action_types(
    document: &Document,
    root: &Object,
    max_depth: usize,
    max_objects: usize,
) -> Result<Vec<String>, &'static str> {
    fn walk(
        document: &Document,
        object: &Object,
        depth: usize,
        max_depth: usize,
        max_objects: usize,
        visited: &mut HashSet<ObjectId>,
        output: &mut Vec<String>,
    ) -> Result<(), &'static str> {
        if depth > max_depth {
            return Err("depth_limit");
        }
        let resolved = match object {
            Object::Reference(id) => {
                if !visited.insert(*id) {
                    return Ok(());
                }
                if visited.len() > max_objects {
                    return Err("object_limit");
                }
                document.get_object(*id).map_err(|_| "dangling_reference")?
            }
            value => value,
        };
        let dictionary = resolved.as_dict().map_err(|_| "not_action_dictionary")?;
        if let Ok(action_type) = dictionary.get(b"S").and_then(Object::as_name) {
            output.push(String::from_utf8_lossy(action_type).into_owned());
        }
        if let Ok(next) = dictionary.get(b"Next") {
            match next {
                Object::Array(actions) => {
                    for action in actions {
                        walk(
                            document,
                            action,
                            depth + 1,
                            max_depth,
                            max_objects,
                            visited,
                            output,
                        )?;
                    }
                }
                action => walk(
                    document,
                    action,
                    depth + 1,
                    max_depth,
                    max_objects,
                    visited,
                    output,
                )?,
            }
        }
        Ok(())
    }

    if matches!(root, Object::Array(_)) {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    walk(
        document,
        root,
        0,
        max_depth,
        max_objects,
        &mut HashSet::new(),
        &mut output,
    )?;
    Ok(output)
}

#[test]
fn manifest_is_strict_complete_ordered_and_digest_bound() {
    let raw = fs::read(fixture_dir().join("manifest.json")).expect("manifest bytes");
    let manifest = read_manifest();
    assert_eq!(manifest.schema_version, "sanitization-fixture-manifest/1");
    assert!(manifest.description.contains("never user data"));
    assert_eq!(manifest.fixtures.len(), 16);

    let mut names = BTreeSet::new();
    for fixture in &manifest.fixtures {
        assert!(names.insert(fixture.name.clone()), "duplicate fixture name");
        assert!(!fixture.purpose.is_empty());
        assert!(!fixture.expected_visual_impact.is_empty());
        assert!(!fixture.expected_functional_impact.is_empty());
        assert!(!fixture.future_transform_expectation.is_empty());
        assert!(!fixture.future_verification_expectation.is_empty());
        let bytes = fs::read(fixture_dir().join(&fixture.name)).expect("fixture bytes");
        assert_eq!(fixture.byte_size, bytes.len());
        assert_eq!(
            fixture.sha256,
            preview_alternative_lib::hex_lower(Sha256::digest(&bytes))
        );
    }

    let mut unknown: serde_json::Value = serde_json::from_slice(&raw).expect("manifest value");
    unknown["fixtures"][0]["unknownField"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<FixtureManifest>(unknown).is_err());

    let mut missing: serde_json::Value = serde_json::from_slice(&raw).expect("manifest value");
    missing["fixtures"][0]
        .as_object_mut()
        .expect("fixture object")
        .remove("purpose");
    assert!(serde_json::from_value::<FixtureManifest>(missing).is_err());
}

#[test]
fn generator_is_byte_for_byte_deterministic() {
    let first = tempdir().expect("first generation dir");
    let second = tempdir().expect("second generation dir");
    let generator = fixture_dir().join("generate_sanitization_fixtures.py");
    for output in [first.path(), second.path()] {
        let result = Command::new("python3")
            .arg(&generator)
            .arg("--output")
            .arg(output)
            .status()
            .expect("run fixture generator");
        assert!(result.success());
    }
    let first_names: BTreeSet<_> = fs::read_dir(first.path())
        .expect("first generated dir")
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let second_names: BTreeSet<_> = fs::read_dir(second.path())
        .expect("second generated dir")
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(first_names, second_names);
    for name in first_names {
        assert_eq!(
            fs::read(first.path().join(&name)).expect("first generated file"),
            fs::read(second.path().join(&name)).expect("second generated file")
        );
    }
}

#[test]
fn parser_supports_classic_incremental_object_xref_and_hybrid_inputs() {
    let classic = load_fixture("javascript-actions.pdf");
    let open_action = classic.catalog().unwrap().get(b"OpenAction").unwrap();
    assert!(matches!(open_action, Object::Reference(_)));
    let page = classic.get_dictionary(classic.get_pages()[&1]).unwrap();
    assert!(matches!(page.get(b"AA").unwrap(), Object::Dictionary(_)));

    let incremental = load_fixture("incremental-revision.pdf");
    assert!(
        !incremental.trailer.has(b"Prev"),
        "lopdf 0.45 consumes /Prev"
    );
    assert!(raw_trailer_history("incremental-revision.pdf").prev);
    assert!(document_contains_marker(&incremental, OLD_REVISION_MARKER));

    let object_stream = load_fixture("object-xref-stream.pdf");
    assert!(object_stream.get_object((5, 0)).is_ok());
    assert!(document_contains_marker(
        &object_stream,
        b"SANITIZATION_OBJECT_STREAM_PAYLOAD_001"
    ));

    let hybrid = load_fixture("hybrid-xref.pdf");
    assert!(hybrid.get_object((5, 0)).is_ok());
    assert!(document_contains_marker(
        &hybrid,
        b"SANITIZATION_HYBRID_OBJECT_001"
    ));
    assert!(!hybrid.trailer.has(b"Prev"), "lopdf 0.45 consumes /Prev");
    assert!(raw_trailer_history("hybrid-xref.pdf").prev);
    assert!(
        !hybrid.trailer.has(b"XRefStm"),
        "lopdf consumes XRefStm while merging it"
    );
    assert!(fs::read(fixture_dir().join("hybrid-xref.pdf"))
        .unwrap()
        .windows(b"/XRefStm".len())
        .any(|window| window == b"/XRefStm"));
}

#[test]
fn incremental_xref_stream_chain_parses_latest_graph_and_retains_physical_history() {
    let path = fixture_dir().join("incremental-xref-stream-chain.pdf");
    let input = fs::read(&path).unwrap();
    assert_eq!(count_occurrences(&input, b"/Type /XRef"), 2);
    assert_eq!(count_occurrences(&input, b"%%EOF"), 2);
    assert!(input
        .windows(XREF_STREAM_OLD_REVISION_MARKER.len())
        .any(|window| window == XREF_STREAM_OLD_REVISION_MARKER));

    let document = Document::load(&path).expect("lopdf parses xref-stream Prev chain");
    assert!(!document.trailer.has(b"Prev"), "lopdf 0.45 consumes /Prev");
    assert!(raw_trailer_history("incremental-xref-stream-chain.pdf").prev);
    assert_eq!(
        document
            .catalog()
            .unwrap()
            .get(b"PageMode")
            .unwrap()
            .as_name()
            .unwrap(),
        b"UseNone"
    );
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_XREF_STREAM_LATEST_REVISION_001"
    ));
    assert!(!document_contains_marker(
        &document,
        XREF_STREAM_OLD_REVISION_MARKER
    ));
}

#[test]
fn bounded_object_graph_walk_handles_cycles_limits_and_dangling_references() {
    let mut document = Document::new();
    let first = document.new_object_id();
    let second = document.new_object_id();
    document.objects.insert(
        first,
        Object::Dictionary(dictionary! { "Next" => Object::Reference(second) }),
    );
    document.objects.insert(
        second,
        Object::Dictionary(dictionary! { "Next" => Object::Reference(first), "Missing" => Object::Reference((999, 0)) }),
    );
    document.trailer.set("Root", Object::Reference(first));

    let evidence = bounded_walk(&document, 16, 100);
    assert_eq!(evidence.indirect_objects, 3);
    assert_eq!(evidence.unresolved_references, vec![(999, 0)]);
    assert!(!evidence.depth_limited);
    assert!(!evidence.object_limited);

    assert!(bounded_walk(&document, 1, 100).depth_limited);
    assert!(bounded_walk(&document, 16, 2).object_limited);
}

#[test]
fn action_chain_traversal_is_type_aware_bounded_and_cycle_safe() {
    let document = load_fixture("action-chain-network.pdf");
    let root = document.catalog().unwrap().get(b"OpenAction").unwrap();
    let actions = collect_action_types(&document, root, 16, 100).expect("bounded action walk");
    assert_eq!(
        actions,
        vec![
            "Launch",
            "URI",
            "SubmitForm",
            "ImportData",
            "GoToR",
            "GoToE"
        ]
    );

    let too_shallow = collect_action_types(&document, root, 1, 100);
    assert_eq!(too_shallow, Err("depth_limit"));

    let destination = load_fixture("openaction-destination.pdf");
    let root = destination.catalog().unwrap().get(b"OpenAction").unwrap();
    assert!(matches!(root, Object::Array(_)));
    assert!(collect_action_types(&destination, root, 16, 100)
        .unwrap()
        .is_empty());
}

#[test]
fn names_tree_branches_are_independently_addressable_and_dests_survive() {
    let document = load_fixture("combined-names.pdf");
    let names = resolve_dictionary(
        &document,
        document.catalog().unwrap().get(b"Names").unwrap(),
    )
    .expect("Names dictionary");
    assert!(names.has(b"Dests"));
    assert!(names.has(b"JavaScript"));
    assert!(names.has(b"EmbeddedFiles"));

    let names_id = document
        .catalog()
        .unwrap()
        .get(b"Names")
        .unwrap()
        .as_reference()
        .unwrap();

    let mut javascript_removed = document.clone();
    javascript_removed
        .get_dictionary_mut(names_id)
        .unwrap()
        .remove(b"JavaScript");
    javascript_removed.prune_objects();
    let names_after = javascript_removed.get_dictionary(names_id).unwrap();
    assert!(names_after.has(b"Dests"));
    assert!(names_after.has(b"EmbeddedFiles"));
    assert!(!names_after.has(b"JavaScript"));
    assert!(!document_contains_marker(
        &javascript_removed,
        b"SANITIZATION_COMBINED_NAMES_JS_001"
    ));
    assert!(document_contains_marker(
        &javascript_removed,
        b"SANITIZATION_COMBINED_NAMES_EMBEDDED_001"
    ));

    let mut embedded_removed = document.clone();
    embedded_removed
        .get_dictionary_mut(names_id)
        .unwrap()
        .remove(b"EmbeddedFiles");
    embedded_removed.prune_objects();
    let names_after = embedded_removed.get_dictionary(names_id).unwrap();
    assert!(names_after.has(b"Dests"));
    assert!(names_after.has(b"JavaScript"));
    assert!(!names_after.has(b"EmbeddedFiles"));
    assert!(document_contains_marker(
        &embedded_removed,
        b"SANITIZATION_COMBINED_NAMES_JS_001"
    ));
    assert!(!document_contains_marker(
        &embedded_removed,
        b"SANITIZATION_COMBINED_NAMES_EMBEDDED_001"
    ));
}

#[test]
fn metadata_paths_and_document_identifier_are_observable_separately() {
    let document = load_fixture("metadata-paths.pdf");
    assert!(document.trailer.has(b"Info"));
    assert!(document.trailer.has(b"ID"));
    let catalog = document.catalog().unwrap();
    assert!(catalog.has(b"Metadata"));
    assert!(catalog.has(b"PieceInfo"));
    let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
    assert!(page.has(b"Metadata"));
    let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
    let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
    let image = resolve_dictionary(&document, xobjects.get(b"Im1").unwrap()).unwrap();
    assert!(image.has(b"Metadata"));
    for marker in [
        b"SANITIZATION_INFO_PAYLOAD_001".as_slice(),
        b"SANITIZATION_XMP_PAYLOAD_001".as_slice(),
        b"SANITIZATION_PAGE_XMP_PAYLOAD_001".as_slice(),
        b"SANITIZATION_OBJECT_XMP_PAYLOAD_001".as_slice(),
    ] {
        assert!(document_contains_marker(&document, marker));
    }
}

#[test]
fn embedded_file_edges_payload_and_privacy_metadata_are_observable() {
    let document = load_fixture("embedded-files-af.pdf");
    let catalog = document.catalog().unwrap();
    assert!(catalog.has(b"AF"));
    assert!(catalog.has(b"Collection"));
    let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
    assert!(page.has(b"AF"));
    let annotation_id = page.get(b"Annots").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let annotation = document.get_dictionary(annotation_id).unwrap();
    assert_eq!(
        annotation.get(b"Subtype").unwrap().as_name().unwrap(),
        b"FileAttachment"
    );
    assert!(annotation.has(b"AF"));
    let file_spec_id = annotation.get(b"FS").unwrap().as_reference().unwrap();
    let file_spec = document.get_dictionary(file_spec_id).unwrap();
    for key in [b"F".as_slice(), b"UF", b"Desc", b"EF", b"RF", b"Params"] {
        assert!(file_spec.has(key), "missing FileSpec key {key:?}");
    }
    let embedded_id = file_spec
        .get(b"EF")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F")
        .unwrap()
        .as_reference()
        .unwrap();
    let embedded = document
        .get_object(embedded_id)
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(
        embedded.dict.get(b"Type").unwrap().as_name().unwrap(),
        b"EmbeddedFile"
    );
    assert!(embedded
        .content
        .windows(EMBEDDED_MARKER.len())
        .any(|w| w == EMBEDDED_MARKER));
    assert_eq!(
        preview_alternative_lib::hex_lower(Sha256::digest(EMBEDDED_MARKER)),
        "90411ddd5f6ac7513cf00a1e983c65418dc62a3cbf343e5c7255071a18ac3cf0"
    );
}

#[test]
fn structure_element_af_resolves_direct_and_indirect_filespecs_without_duplicate_inflation() {
    let document = load_fixture("embedded-files-af.pdf");
    let struct_root = resolve_dictionary(
        &document,
        document.catalog().unwrap().get(b"StructTreeRoot").unwrap(),
    )
    .expect("StructTreeRoot");
    let struct_elem =
        resolve_dictionary(&document, struct_root.get(b"K").unwrap()).expect("Structure Element");
    assert_eq!(
        struct_elem.get(b"Type").unwrap().as_name().unwrap(),
        b"StructElem"
    );
    let af = struct_elem.get(b"AF").unwrap().as_array().unwrap();
    assert_eq!(af.len(), 2);
    assert!(matches!(af[0], Object::Reference((7, 0))));
    assert_eq!(
        resolve_dictionary(&document, &af[0])
            .unwrap()
            .get(b"Type")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Filespec"
    );
    assert!(matches!(af[1], Object::Dictionary(_)));
    let direct_file_spec = resolve_dictionary(&document, &af[1]).unwrap();
    let direct_stream = direct_file_spec
        .get(b"EF")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F")
        .unwrap()
        .as_reference()
        .unwrap();
    assert!(document
        .get_object(direct_stream)
        .unwrap()
        .as_stream()
        .is_ok());

    let catalog = document.catalog().unwrap();
    let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
    let annotation = resolve_dictionary(
        &document,
        &page.get(b"Annots").unwrap().as_array().unwrap()[0],
    )
    .unwrap();
    let owners = [catalog, page, annotation, struct_elem];
    let mut indirect_filespec_ids = BTreeSet::new();
    let mut indirect_edge_count = 0;
    for owner in owners {
        for item in owner.get(b"AF").unwrap().as_array().unwrap() {
            if let Ok(id) = item.as_reference() {
                indirect_edge_count += 1;
                indirect_filespec_ids.insert(id);
            }
        }
    }
    assert_eq!(indirect_edge_count, 4);
    let embedded_name_tree = resolve_dictionary(
        &document,
        catalog
            .get(b"Names")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"EmbeddedFiles")
            .unwrap(),
    )
    .unwrap();
    let name_tree_file_spec = embedded_name_tree
        .get(b"Names")
        .unwrap()
        .as_array()
        .unwrap()[1]
        .as_reference()
        .unwrap();
    indirect_edge_count += 1;
    indirect_filespec_ids.insert(name_tree_file_spec);
    assert_eq!(indirect_edge_count, 5);
    assert_eq!(indirect_filespec_ids, BTreeSet::from([(7, 0)]));

    let mut structure_only = document.clone();
    structure_only
        .catalog_mut()
        .unwrap()
        .get_mut(b"Names")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"EmbeddedFiles");
    structure_only.catalog_mut().unwrap().remove(b"AF");
    let page_id = structure_only.get_pages()[&1];
    structure_only
        .get_dictionary_mut(page_id)
        .unwrap()
        .remove(b"AF");
    structure_only
        .get_dictionary_mut(page_id)
        .unwrap()
        .remove(b"Annots");
    structure_only.prune_objects();
    assert!(structure_only.get_object((7, 0)).is_ok());
    assert!(structure_only.get_object((8, 0)).is_ok());
    assert!(document_contains_marker(&structure_only, EMBEDDED_MARKER));
}

#[test]
fn unsupported_xfa_optional_content_and_multimedia_paths_are_detectable() {
    let document = load_fixture("unsupported-active-content.pdf");
    let catalog = document.catalog().unwrap();
    assert!(catalog.has(b"AcroForm"));
    assert!(catalog.has(b"OCProperties"));
    let acroform = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
    let xfa = acroform.get(b"XFA").unwrap().as_array().unwrap();
    assert_eq!(xfa.len(), 4);
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_XFA_PAYLOAD_001"
    ));
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_OPTIONAL_CONTENT_001"
    ));

    let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
    let subtype_names: BTreeSet<Vec<u8>> = page
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            resolve_dictionary(&document, item)
                .unwrap()
                .get(b"Subtype")
                .unwrap()
                .as_name()
                .unwrap()
                .to_vec()
        })
        .collect();
    for subtype in [
        b"RichMedia".as_slice(),
        b"Screen",
        b"Movie",
        b"Sound",
        b"3D",
    ] {
        assert!(subtype_names.contains(subtype));
    }
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_RICHMEDIA_PAYLOAD_001"
    ));
}

#[test]
fn xfa_single_reference_resolves_stream_and_prunes_after_edge_removal() {
    let mut document = load_fixture("xfa-single-stream.pdf");
    let acroform = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_dict()
        .unwrap();
    let xfa = acroform.get(b"XFA").unwrap();
    assert!(matches!(xfa, Object::Reference((5, 0))));
    let stream = resolve_object(&document, xfa).unwrap().as_stream().unwrap();
    assert!(stream
        .content
        .windows(b"SANITIZATION_XFA_SINGLE_STREAM_001".len())
        .any(|window| window == b"SANITIZATION_XFA_SINGLE_STREAM_001"));

    document
        .catalog_mut()
        .unwrap()
        .get_mut(b"AcroForm")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"XFA");
    let pruned = document.prune_objects();
    assert!(pruned.contains(&(5, 0)));
    assert!(!document_contains_marker(
        &document,
        b"SANITIZATION_XFA_SINGLE_STREAM_001"
    ));
}

#[test]
fn xfa_array_references_resolve_streams_and_prune_after_edge_removal() {
    let mut document = load_fixture("unsupported-active-content.pdf");
    let xfa = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"XFA")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(xfa.len(), 4);
    assert_eq!(xfa[0].as_str().unwrap(), b"template");
    assert!(resolve_object(&document, &xfa[1])
        .unwrap()
        .as_stream()
        .is_ok());
    assert_eq!(xfa[2].as_str().unwrap(), b"datasets");
    assert!(resolve_object(&document, &xfa[3])
        .unwrap()
        .as_stream()
        .is_ok());
    document
        .catalog_mut()
        .unwrap()
        .get_mut(b"AcroForm")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"XFA");
    let pruned = document.prune_objects();
    assert!(pruned.contains(&(6, 0)));
    assert!(pruned.contains(&(7, 0)));
    assert!(!document_contains_marker(
        &document,
        b"SANITIZATION_XFA_PAYLOAD_001"
    ));
}

#[test]
fn optional_content_ocg_ocmd_and_annotation_oc_are_independently_detectable() {
    let document = load_fixture("unsupported-active-content.pdf");
    let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
    let properties = page
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Properties")
        .unwrap()
        .as_dict()
        .unwrap();
    let ocg = resolve_dictionary(&document, properties.get(b"Layer1").unwrap()).unwrap();
    let ocmd = resolve_dictionary(&document, properties.get(b"Membership").unwrap()).unwrap();
    assert_eq!(ocg.get(b"Type").unwrap().as_name().unwrap(), b"OCG");
    assert_eq!(ocmd.get(b"Type").unwrap().as_name().unwrap(), b"OCMD");
    assert!(ocmd.has(b"OCGs"));

    let layer_annotation = page
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| resolve_dictionary(&document, item).unwrap())
        .find(|annotation| annotation.has(b"OC"))
        .expect("annotation with optional-content association");
    assert_eq!(
        layer_annotation.get(b"OC").unwrap().as_reference().unwrap(),
        (15, 0)
    );
    assert!(
        document.catalog().unwrap().has(b"OCProperties"),
        "Catalog visibility state is separate from page resource and annotation edges"
    );
}

#[test]
fn removing_ocproperties_does_not_remove_optional_marked_content() {
    let mut document = load_fixture("unsupported-active-content.pdf");
    document.catalog_mut().unwrap().remove(b"OCProperties");
    document.prune_objects();
    assert!(!document.catalog().unwrap().has(b"OCProperties"));
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_OPTIONAL_CONTENT_001"
    ));
    assert!(document_contains_marker(
        &document,
        b"SANITIZATION_OCMD_CONTENT_001"
    ));
    assert_eq!(
        document
            .get_dictionary((8, 0))
            .unwrap()
            .get(b"Type")
            .unwrap()
            .as_name()
            .unwrap(),
        b"OCG"
    );
    assert_eq!(
        document
            .get_dictionary((15, 0))
            .unwrap()
            .get(b"Type")
            .unwrap()
            .as_name()
            .unwrap(),
        b"OCMD"
    );
    let annotation = document.get_dictionary((16, 0)).unwrap();
    assert_eq!(
        annotation.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Text"
    );
    assert_eq!(
        annotation.get(b"OC").unwrap().as_reference().unwrap(),
        (15, 0)
    );
}

#[test]
fn embedded_name_tree_removal_alone_leaves_af_payload_but_all_edges_allow_pruning() {
    let mut partial = load_fixture("embedded-files-af.pdf");
    partial
        .catalog_mut()
        .unwrap()
        .get_mut(b"Names")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"EmbeddedFiles");
    partial.prune_objects();
    assert!(document_contains_marker(&partial, EMBEDDED_MARKER));

    let mut complete = load_fixture("embedded-files-af.pdf");
    complete
        .catalog_mut()
        .unwrap()
        .get_mut(b"Names")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"EmbeddedFiles");
    complete.catalog_mut().unwrap().remove(b"AF");
    complete.catalog_mut().unwrap().remove(b"StructTreeRoot");
    let page_id = complete.get_pages()[&1];
    complete.get_dictionary_mut(page_id).unwrap().remove(b"AF");
    complete
        .get_dictionary_mut(page_id)
        .unwrap()
        .remove(b"Annots");
    let pruned = complete.prune_objects();
    assert!(pruned.contains(&(7, 0)));
    assert!(pruned.contains(&(8, 0)));
    assert!(pruned.contains(&(10, 0)));
    assert!(pruned.contains(&(11, 0)));
    assert!(pruned.contains(&(12, 0)));
    assert!(!document_contains_marker(&complete, EMBEDDED_MARKER));
}

#[test]
fn pruning_removes_unreachable_payloads_but_retains_reachable_and_shared_objects() {
    let mut document = Document::new();
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => (2, 0) });
    let pages_id = document.add_object(
        dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference((3, 0))], "Count" => 1 },
    );
    let shared_id = document.add_object(Object::String(
        b"SANITIZATION_SHARED_REACHABLE_001".to_vec(),
        StringFormat::Literal,
    ));
    let reachable_id = document.add_object(dictionary! { "Shared" => shared_id });
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "FixtureA" => reachable_id,
    });
    assert_eq!(catalog_id, (1, 0));
    assert_eq!(pages_id, (2, 0));
    assert_eq!(page_id, (5, 0));
    document
        .get_dictionary_mut(pages_id)
        .unwrap()
        .set("Kids", vec![Object::Reference(page_id)]);
    let sensitive_id = document.add_object(Stream::new(
        Dictionary::new(),
        b"SANITIZATION_UNREACHABLE_STREAM_001".to_vec(),
    ));
    let metadata_id = document.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"SANITIZATION_UNREACHABLE_METADATA_001".to_vec(),
    ));
    document.trailer.set("Root", catalog_id);

    let pruned = document.prune_objects();
    assert!(pruned.contains(&sensitive_id));
    assert!(pruned.contains(&metadata_id));
    assert!(!pruned.contains(&reachable_id));
    assert!(!pruned.contains(&shared_id));

    let dir = tempdir().unwrap();
    let output = dir.path().join("pruned.pdf");
    document.save(&output).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert!(!bytes
        .windows(b"SANITIZATION_UNREACHABLE_STREAM_001".len())
        .any(|w| w == b"SANITIZATION_UNREACHABLE_STREAM_001"));
    assert!(!bytes
        .windows(b"SANITIZATION_UNREACHABLE_METADATA_001".len())
        .any(|w| w == b"SANITIZATION_UNREACHABLE_METADATA_001"));
    assert!(bytes
        .windows(b"SANITIZATION_SHARED_REACHABLE_001".len())
        .any(|w| w == b"SANITIZATION_SHARED_REACHABLE_001"));
    let reopened = Document::load(&output).unwrap();
    assert!(reopened.get_object(reachable_id).is_ok());
    assert!(reopened.get_object(shared_id).is_ok());
}

/// lopdf 0.34 copied the input's stale /Prev into a full save, so the transform had to
/// remove it explicitly (it still does). 0.45 drops /Prev on load: a full save carries no
/// history pointer and reopens.
#[test]
fn lopdf_full_save_does_not_copy_a_stale_prev() {
    let mut document = load_fixture("incremental-revision.pdf");
    assert!(!document.trailer.has(b"Prev"));
    let dir = tempdir().unwrap();
    let output = dir.path().join("no-stale-prev.pdf");
    document.save(&output).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert!(!bytes
        .windows(b"/Prev".len())
        .any(|window| window == b"/Prev"));
    let reopened = Document::load_mem(&bytes).expect("full save reopens");
    assert_eq!(
        preview_alternative_lib::lopdf_compat::trailer_history(&bytes, &reopened),
        Some(preview_alternative_lib::lopdf_compat::TrailerHistory::default())
    );
}

#[test]
fn explicit_prune_and_full_rewrite_removes_history_and_payload() {
    let input_path = fixture_dir().join("incremental-revision.pdf");
    let input = fs::read(&input_path).unwrap();
    let mut document = Document::load(&input_path).unwrap();
    document.catalog_mut().unwrap().remove(b"OpenAction");
    document.catalog_mut().unwrap().remove(b"Metadata");
    document.trailer.remove(b"Prev");
    document.trailer.remove(b"XRefStm");
    let pruned = document.prune_objects();
    assert!(
        pruned.contains(&(5, 0)),
        "old JavaScript object must become unreachable"
    );
    assert!(
        pruned.contains(&(6, 0)),
        "current metadata stream must become unreachable"
    );

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("full-rewrite.pdf");
    document.save(&output_path).unwrap();
    let output = fs::read(&output_path).unwrap();
    assert!(!output.starts_with(&input));
    assert_eq!(count_occurrences(&output, b"%%EOF"), 1);
    assert!(!output
        .windows(OLD_REVISION_MARKER.len())
        .any(|w| w == OLD_REVISION_MARKER));
    assert!(!output
        .windows(b"SANITIZATION_INCREMENTAL_XMP_001".len())
        .any(|w| w == b"SANITIZATION_INCREMENTAL_XMP_001"));
    assert!(!output.windows(b"/Prev".len()).any(|w| w == b"/Prev"));

    let reopened = Document::load(&output_path).unwrap();
    assert!(!reopened.trailer.has(b"Prev"));
    assert!(!reopened.trailer.has(b"XRefStm"));
    assert!(!document_contains_marker(&reopened, OLD_REVISION_MARKER));
    assert_eq!(reopened.get_pages().len(), 1);

    let (in_use_entries, free_entries) = validate_classic_xref_points_to_output_objects(&output);
    assert_eq!(in_use_entries, reopened.objects.len());
    assert_eq!(free_entries, 1);
    assert_ne!(parse_startxref(&input), parse_startxref(&output));

    let _guard = PDFIUM_TEST_MUTEX.lock();
    let pdfium = preview_alternative_lib::pdf_engine::get_pdfium().expect("initialize PDFium");
    let pdfium_output = pdfium
        .load_pdf_from_file(&output_path, None)
        .expect("PDFium reopens full rewrite output");
    assert_eq!(pdfium_output.pages().len(), 1);
    pdfium_output
        .pages()
        .get(0)
        .unwrap()
        .render_with_config(&PdfRenderConfig::new().set_target_width(64))
        .expect("PDFium renders full rewrite output");
}

#[test]
fn xref_stream_chain_full_rewrite_normalizes_history_and_uses_output_local_offsets() {
    let input_path = fixture_dir().join("incremental-xref-stream-chain.pdf");
    let input = fs::read(&input_path).unwrap();
    let mut document = Document::load(&input_path).unwrap();
    assert!(raw_trailer_history("incremental-xref-stream-chain.pdf").prev);
    document.catalog_mut().unwrap().remove(b"Metadata");
    document.trailer.remove(b"Prev");
    document.trailer.remove(b"XRefStm");
    let pruned = document.prune_objects();
    assert!(pruned.contains(&(5, 0)));

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("xref-stream-chain-full-rewrite.pdf");
    document.save(&output_path).unwrap();
    let output = fs::read(&output_path).unwrap();
    assert!(!output.starts_with(&input));
    assert_eq!(count_occurrences(&output, b"%%EOF"), 1);
    assert!(!output
        .windows(b"/Prev".len())
        .any(|window| window == b"/Prev"));
    assert!(!output
        .windows(XREF_STREAM_OLD_REVISION_MARKER.len())
        .any(|window| window == XREF_STREAM_OLD_REVISION_MARKER));
    assert!(!output
        .windows(b"SANITIZATION_XREF_STREAM_LATEST_REVISION_001".len())
        .any(|window| window == b"SANITIZATION_XREF_STREAM_LATEST_REVISION_001"));

    let reopened = Document::load(&output_path).expect("reopen normalized xref rewrite");
    assert!(!reopened.trailer.has(b"Prev"));
    assert_eq!(
        reopened
            .catalog()
            .unwrap()
            .get(b"PageMode")
            .unwrap()
            .as_name()
            .unwrap(),
        b"UseNone"
    );
    assert_eq!(reopened.get_pages().len(), 1);
    let in_use_entries = validate_xref_stream_points_to_output_objects(&output);
    assert_eq!(in_use_entries, reopened.objects.len());
    assert_ne!(parse_startxref(&input), parse_startxref(&output));

    let _guard = PDFIUM_TEST_MUTEX.lock();
    let pdfium = preview_alternative_lib::pdf_engine::get_pdfium().expect("initialize PDFium");
    let pdfium_output = pdfium
        .load_pdf_from_file(&output_path, None)
        .expect("PDFium reopens xref-stream-chain rewrite");
    assert_eq!(pdfium_output.pages().len(), 1);
    pdfium_output
        .pages()
        .get(0)
        .unwrap()
        .render_with_config(&PdfRenderConfig::new().set_target_width(64))
        .expect("PDFium renders xref-stream-chain rewrite");
}

#[test]
fn document_id_is_preserved_or_removed_only_by_explicit_trailer_policy() {
    let source = load_fixture("metadata-paths.pdf");
    let original_id = source.trailer.get(b"ID").unwrap().clone();
    let dir = tempdir().unwrap();

    let mut preserved = source.clone();
    let preserved_path = dir.path().join("id-preserved.pdf");
    preserved.save(&preserved_path).unwrap();
    assert_eq!(
        Document::load(&preserved_path)
            .unwrap()
            .trailer
            .get(b"ID")
            .unwrap(),
        &original_id
    );

    let mut removed = source.clone();
    removed.trailer.remove(b"ID");
    let removed_path = dir.path().join("id-removed.pdf");
    removed.save(&removed_path).unwrap();
    assert!(!Document::load(&removed_path).unwrap().trailer.has(b"ID"));

    let mut generated = Document::with_version("1.7");
    let catalog = generated.add_object(dictionary! { "Type" => "Catalog", "Pages" => (2, 0) });
    generated.add_object(
        dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
    );
    generated.trailer.set("Root", catalog);
    let generated_path = dir.path().join("no-writer-id.pdf");
    generated.save(&generated_path).unwrap();
    assert!(!Document::load(&generated_path).unwrap().trailer.has(b"ID"));
}

#[test]
fn reachable_dangling_critical_references_open_but_block_graph_completeness() {
    let document = load_fixture("dangling-critical-references.pdf");
    assert_eq!(document.get_pages().len(), 1, "parser-open succeeds");
    assert!(document.catalog().unwrap().has(b"OpenAction"));
    assert!(document.catalog().unwrap().has(b"Names"));
    assert!(document.get_object((98, 0)).is_err());
    assert!(document.get_object((99, 0)).is_err());
    let unresolved: BTreeSet<_> = bounded_walk(&document, 16, 100)
        .unresolved_references
        .into_iter()
        .collect();
    assert_eq!(unresolved, BTreeSet::from([(98, 0), (99, 0)]));
}

/// lopdf 0.45: an encrypted file must get its password while loading. Loaded without it,
/// the objects in encrypted object streams (the catalog included) are never parsed and a
/// later `decrypt` cannot bring them back, so nothing may load first and decrypt after
/// (production uses `lopdf_compat::load_mem_with_credential`). A decrypted save is plain.
#[test]
fn encrypted_input_requires_credentials_and_decrypt_save_becomes_unencrypted() {
    let bytes = fs::read(fixture_dir().join("encrypted.pdf")).unwrap();
    let marker = b"SANITIZATION_ENCRYPTED_PAYLOAD_001";

    let mut without_password = Document::load_mem(&bytes).expect("parse encrypted xref");
    assert!(without_password.is_encrypted());
    assert!(without_password.catalog().is_err());
    without_password.decrypt_raw(b"fixture-password").unwrap();
    assert!(
        without_password.catalog().is_err(),
        "decrypting after load does not recover object-stream objects"
    );

    let with_password = |password: &str| {
        Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(password))
    };
    assert!(with_password("wrong-password").is_err());
    let encrypted = with_password("fixture-password").expect("load with the password");
    assert!(!encrypted.is_encrypted());
    assert!(encrypted.was_encrypted());
    assert!(document_contains_marker(&encrypted, marker));

    let dir = tempdir().unwrap();
    let decrypted_path = dir.path().join("decrypted-rewrite.pdf");
    encrypted.clone().save(&decrypted_path).unwrap();
    let decrypted = Document::load(&decrypted_path).unwrap();
    assert!(!decrypted.is_encrypted());
    assert!(!decrypted.was_encrypted());
    assert!(fs::read(&decrypted_path)
        .unwrap()
        .windows(marker.len())
        .any(|window| window == marker));
}

#[test]
fn signature_byte_range_docmdp_and_fieldmdp_are_structurally_visible() {
    let document = load_fixture("signature.pdf");
    let catalog = document.catalog().unwrap();
    assert!(catalog.has(b"Perms"));
    let acroform = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
    let field_id = acroform.get(b"Fields").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let field = document.get_dictionary(field_id).unwrap();
    assert_eq!(field.get(b"FT").unwrap().as_name().unwrap(), b"Sig");
    let signature_id = field.get(b"V").unwrap().as_reference().unwrap();
    let signature = document.get_dictionary(signature_id).unwrap();
    assert_eq!(signature.get(b"Type").unwrap().as_name().unwrap(), b"Sig");
    assert_eq!(
        signature
            .get(b"ByteRange")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let references = signature.get(b"Reference").unwrap().as_array().unwrap();
    let methods: BTreeSet<Vec<u8>> = references
        .iter()
        .map(|item| {
            item.as_dict()
                .unwrap()
                .get(b"TransformMethod")
                .unwrap()
                .as_name()
                .unwrap()
                .to_vec()
        })
        .collect();
    assert!(methods.contains(b"DocMDP".as_slice()));
    assert!(methods.contains(b"FieldMDP".as_slice()));
}

#[test]
fn pdfium_reopens_renders_and_reports_basic_facts_without_object_mutation_authority() {
    let _guard = PDFIUM_TEST_MUTEX.lock();
    let pdfium = preview_alternative_lib::pdf_engine::get_pdfium().expect("initialize PDFium");
    for name in [
        "clean-minimal.pdf",
        "metadata-paths.pdf",
        "javascript-actions.pdf",
        "embedded-files-af.pdf",
        "object-xref-stream.pdf",
        "hybrid-xref.pdf",
        "signature.pdf",
    ] {
        let document = pdfium
            .load_pdf_from_file(&fixture_dir().join(name), None)
            .unwrap_or_else(|error| panic!("PDFium failed to reopen {name}: {error:?}"));
        assert_eq!(document.pages().len(), 1, "{name}");
        let page = document.pages().get(0).unwrap();
        assert!(page.width().value > 0.0);
        assert!(page.height().value > 0.0);
        page.render_with_config(&PdfRenderConfig::new().set_target_width(64))
            .unwrap_or_else(|error| panic!("PDFium render smoke failed for {name}: {error:?}"));
    }

    let signed = pdfium
        .load_pdf_from_file(&fixture_dir().join("signature.pdf"), None)
        .unwrap();
    assert_eq!(signed.signatures().len(), 1);

    assert!(pdfium
        .load_pdf_from_file(&fixture_dir().join("encrypted.pdf"), None)
        .is_err());
    let encrypted = pdfium
        .load_pdf_from_file(
            &fixture_dir().join("encrypted.pdf"),
            Some("fixture-password"),
        )
        .expect("PDFium credentialed encrypted reopen");
    assert_eq!(encrypted.pages().len(), 1);
    assert_eq!(
        encrypted.permissions().security_handler_revision().unwrap(),
        PdfSecurityHandlerRevision::Revision2
    );
}
