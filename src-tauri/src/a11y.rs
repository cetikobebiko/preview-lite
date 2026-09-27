use lopdf::{decode_text_string, Dictionary, Document, Object, ObjectId};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const ACCESSIBILITY_AUDIT_SCHEMA_VERSION: &str = "accessibility-audit/2";
pub const ACCESSIBILITY_AUDIT_VERSION: &str = "basic-accessibility-audit/1";
pub const ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION: &str = "accessibility-structure/1";

const MAX_STRUCTURE_DEPTH: usize = 128;
const MAX_STRUCTURE_NODES: usize = 10_000;
const MAX_VISITED_STRUCTURE_OBJECTS: usize = 8_192;
const MAX_CHILDREN_PER_NODE: usize = 4_096;
const MAX_EVIDENCE_PER_CHECK: usize = 32;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_CHARS: usize = 512;
const MAX_XMP_BYTES: usize = 1024 * 1024;
const MAX_REPORT_BYTES: usize = 1024 * 1024;
const MAX_PDF_OBJECTS: usize = 100_000;
const MAX_ROLE_MAP_ENTRIES: usize = 4_096;
const MAX_PARENT_TREE_ENTRIES: usize = 50_000;
const MAX_NUMBER_TREE_DEPTH: usize = 64;
const COOPERATIVE_DEADLINE: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityReferenceProfile {
    PdfUa1Reference,
    PdfUa2Reference,
    Undetermined,
    UnsupportedPdfVersion,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilitySummaryStatus {
    FindingsDetected,
    NoFindingsInImplementedChecks,
    AuditIncomplete,
    AuditFailed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityCheckResult {
    Pass,
    Fail,
    Warning,
    NotApplicable,
    NotChecked,
    Indeterminate,
    HumanReviewRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilitySeverity {
    Info,
    Minor,
    Major,
    Critical,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityAutomation {
    Machine,
    Mixed,
    Human,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityTextState {
    #[default]
    Missing,
    Empty,
    WhitespaceOnly,
    Present,
    Invalid,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityEvidence {
    pub object_id: Option<String>,
    pub page_index: Option<u32>,
    pub structure_path: Option<String>,
    pub field: Option<String>,
    pub message: String,
    pub value_preview: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityStandardReference {
    pub standard: String,
    pub clause: String,
    pub reference_type: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityCheck {
    pub id: String,
    pub title: String,
    pub category: String,
    pub result: AccessibilityCheckResult,
    pub severity: AccessibilitySeverity,
    pub automation: AccessibilityAutomation,
    pub evidence: Vec<AccessibilityEvidence>,
    pub standard_references: Vec<AccessibilityStandardReference>,
    pub conformance_claim: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfUaDeclaration {
    pub part: Option<String>,
    pub revision: Option<String>,
    pub state: AccessibilityTextState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityDocumentFacts {
    pub header_version: Option<String>,
    pub catalog_version: Option<String>,
    pub effective_version: Option<String>,
    pub pdfua_declaration: PdfUaDeclaration,
    pub page_count: u32,
    pub encrypted: bool,
    pub has_structure_tree: bool,
    pub has_mark_info: bool,
    pub has_marked_entry: bool,
    pub marked: bool,
    pub display_document_title: bool,
    pub document_language: AccessibilityTextState,
    pub xmp_title: AccessibilityTextState,
    pub info_title: AccessibilityTextState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityCategoryCoverage {
    pub category: String,
    pub status: String,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityCoverage {
    pub implemented_check_ids: Vec<String>,
    pub not_implemented_check_ids: Vec<String>,
    pub incomplete_check_ids: Vec<String>,
    pub machine_check_count: u32,
    pub human_review_count: u32,
    pub categories: Vec<AccessibilityCategoryCoverage>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityHumanReviewItem {
    pub id: String,
    pub title: String,
    pub category: String,
    pub result: AccessibilityCheckResult,
    pub applies: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityAuditReport {
    pub schema_version: String,
    pub audit_version: String,
    pub reference_profile: AccessibilityReferenceProfile,
    pub summary_status: AccessibilitySummaryStatus,
    pub document_facts: AccessibilityDocumentFacts,
    pub checks: Vec<AccessibilityCheck>,
    pub coverage: AccessibilityCoverage,
    pub human_review: Vec<AccessibilityHumanReviewItem>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityStructureNodeKind {
    DocumentRoot,
    StructureElement,
    MarkedContentReference,
    ObjectReference,
    Mcid,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityStructureNode {
    pub id: String,
    pub kind: AccessibilityStructureNodeKind,
    pub raw_type: Option<String>,
    pub mapped_type: Option<String>,
    pub object_id: Option<String>,
    pub page_index: Option<u32>,
    pub mcid: Option<i64>,
    pub alt_text_state: AccessibilityTextState,
    pub alt_text_preview: Option<String>,
    pub actual_text_state: AccessibilityTextState,
    pub actual_text_preview: Option<String>,
    pub language_state: AccessibilityTextState,
    pub language_preview: Option<String>,
    pub association_status: String,
    pub children: Vec<AccessibilityStructureNode>,
    pub findings: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityStructureInspection {
    pub schema_version: String,
    pub inspection_version: String,
    pub root: Option<AccessibilityStructureNode>,
    pub findings: Vec<String>,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Default)]
struct XmpFacts {
    part: Option<String>,
    revision: Option<String>,
    title: Option<String>,
    state: AccessibilityTextState,
    warning: Option<String>,
}

struct TraversalBudget {
    started: Instant,
    nodes: usize,
    visited_objects: HashSet<ObjectId>,
    truncated: bool,
    warnings: Vec<String>,
}

impl TraversalBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            nodes: 0,
            visited_objects: HashSet::new(),
            truncated: false,
            warnings: Vec::new(),
        }
    }
    fn admit(&mut self, depth: usize, object_id: Option<ObjectId>) -> bool {
        let reason = if depth > MAX_STRUCTURE_DEPTH {
            Some("Structure depth limit reached")
        } else if object_id.is_some_and(|id| {
            !self.visited_objects.contains(&id)
                && self.visited_objects.len() >= MAX_VISITED_STRUCTURE_OBJECTS
        }) {
            Some("Structure visited-object limit reached")
        } else if self.nodes >= MAX_STRUCTURE_NODES {
            Some("Structure node limit reached")
        } else if self.started.elapsed() >= COOPERATIVE_DEADLINE {
            Some("Accessibility inspection cooperative deadline reached")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.truncated = true;
            if !self.warnings.iter().any(|warning| warning == reason) {
                self.warnings.push(reason.to_string());
            }
            return false;
        }
        if let Some(id) = object_id {
            self.visited_objects.insert(id);
        }
        self.nodes += 1;
        true
    }
}

#[derive(Clone)]
struct PendingNode {
    object: Object,
    object_id: Option<ObjectId>,
    path: String,
    depth: usize,
    ancestors: HashSet<ObjectId>,
}

fn object_id_string(id: ObjectId) -> String {
    format!("{} {} R", id.0, id.1)
}

fn bounded_preview(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .take(MAX_PREVIEW_CHARS)
        .collect()
}

fn text_value(object: Option<&Object>) -> (AccessibilityTextState, Option<String>) {
    let Some(object) = object else {
        return (AccessibilityTextState::Missing, None);
    };
    let Ok(bytes) = object.as_str() else {
        return (AccessibilityTextState::Invalid, None);
    };
    if bytes.len() > MAX_TEXT_BYTES {
        return (AccessibilityTextState::Invalid, None);
    }
    match decode_text_string(object) {
        Ok(value) if value.is_empty() => (AccessibilityTextState::Empty, Some(String::new())),
        Ok(value) if value.trim().is_empty() => (
            AccessibilityTextState::WhitespaceOnly,
            Some(bounded_preview(&value)),
        ),
        Ok(value) => (
            AccessibilityTextState::Present,
            Some(bounded_preview(&value)),
        ),
        Err(_) => (AccessibilityTextState::Invalid, None),
    }
}

fn name_value(object: Option<&Object>) -> Option<String> {
    object
        .and_then(|value| value.as_name().ok())
        .map(|value| bounded_preview(&String::from_utf8_lossy(value)))
}

fn resolve_object(doc: &Document, object: &Object) -> Result<(Object, Option<ObjectId>), String> {
    match object {
        Object::Reference(id) => doc
            .get_object(*id)
            .cloned()
            .map(|object| (object, Some(*id)))
            .map_err(|_| format!("Dangling structure reference: {}", object_id_string(*id))),
        _ => Ok((object.clone(), None)),
    }
}

fn direct_dictionary<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        Object::Reference(id) => doc
            .get_object(*id)
            .ok()
            .and_then(|value| direct_dictionary(doc, value)),
        _ => None,
    }
}

fn empty_node(
    id: String,
    object_id: Option<ObjectId>,
    kind: AccessibilityStructureNodeKind,
    finding: Option<String>,
) -> AccessibilityStructureNode {
    AccessibilityStructureNode {
        id,
        kind,
        raw_type: None,
        mapped_type: None,
        object_id: object_id.map(object_id_string),
        page_index: None,
        mcid: None,
        alt_text_state: AccessibilityTextState::Missing,
        alt_text_preview: None,
        actual_text_state: AccessibilityTextState::Missing,
        actual_text_preview: None,
        language_state: AccessibilityTextState::Missing,
        language_preview: None,
        association_status: "unresolved".to_string(),
        children: Vec::new(),
        findings: finding.into_iter().collect(),
        truncated: false,
    }
}

fn build_node(
    doc: &Document,
    pending: PendingNode,
    budget: &mut TraversalBudget,
    page_indices: &HashMap<ObjectId, u32>,
) -> AccessibilityStructureNode {
    let occurrence_id = pending
        .object_id
        .map(|id| format!("obj:{}:{}@{}", id.0, id.1, pending.path))
        .unwrap_or_else(|| format!("path:{}", pending.path));
    if !budget.admit(pending.depth, pending.object_id) {
        let mut node = empty_node(
            occurrence_id,
            pending.object_id,
            AccessibilityStructureNodeKind::Unknown,
            Some("Node omitted because an inspection budget was reached".to_string()),
        );
        node.truncated = true;
        return node;
    }
    if let Object::Integer(mcid) = pending.object {
        let mut node = empty_node(
            occurrence_id,
            pending.object_id,
            AccessibilityStructureNodeKind::Mcid,
            None,
        );
        node.mcid = Some(mcid);
        node.association_status = "partial".to_string();
        return node;
    }
    let Some(dictionary) = direct_dictionary(doc, &pending.object) else {
        return empty_node(
            occurrence_id,
            pending.object_id,
            AccessibilityStructureNodeKind::Unknown,
            Some("Unsupported structure child object type".to_string()),
        );
    };
    let kind = match name_value(dictionary.get(b"Type").ok()).as_deref() {
        Some("StructTreeRoot") => AccessibilityStructureNodeKind::DocumentRoot,
        Some("MCR") => AccessibilityStructureNodeKind::MarkedContentReference,
        Some("OBJR") => AccessibilityStructureNodeKind::ObjectReference,
        Some("StructElem") | None => AccessibilityStructureNodeKind::StructureElement,
        _ => AccessibilityStructureNodeKind::Unknown,
    };
    let raw_type =
        name_value(dictionary.get(b"S").ok()).or_else(|| name_value(dictionary.get(b"Type").ok()));
    let (alt_text_state, alt_text_preview) = text_value(dictionary.get(b"Alt").ok());
    let (actual_text_state, actual_text_preview) = text_value(dictionary.get(b"ActualText").ok());
    let (language_state, language_preview) = text_value(dictionary.get(b"Lang").ok());
    let mcid = dictionary.get(b"MCID").and_then(Object::as_i64).ok();
    let mut findings = Vec::new();
    let mut children = Vec::new();
    if matches!(kind, AccessibilityStructureNodeKind::MarkedContentReference) {
        if mcid.is_none() {
            findings.push("MCR is missing a numeric MCID".to_string());
        }
        if let Ok(Object::Reference(id)) = dictionary.get(b"Pg") {
            if doc.get_object(*id).is_err() {
                findings.push(format!(
                    "MCR has dangling Pg reference {}",
                    object_id_string(*id)
                ));
            }
        }
    }
    if matches!(kind, AccessibilityStructureNodeKind::ObjectReference) {
        match dictionary.get(b"Obj") {
            Ok(Object::Reference(id)) if doc.get_object(*id).is_err() => findings.push(format!(
                "OBJR has dangling Obj reference {}",
                object_id_string(*id)
            )),
            Ok(Object::Reference(_)) => {}
            _ => findings.push("OBJR is missing a resolvable Obj reference".to_string()),
        }
    }
    if matches!(
        kind,
        AccessibilityStructureNodeKind::StructureElement
            | AccessibilityStructureNodeKind::DocumentRoot
    ) {
        if let Ok(kids) = dictionary.get(b"K") {
            let candidates: Vec<&Object> = match kids {
                Object::Array(values) => {
                    if values.len() > MAX_CHILDREN_PER_NODE {
                        budget.truncated = true;
                        findings.push(format!(
                            "Child count {} exceeds limit {}",
                            values.len(),
                            MAX_CHILDREN_PER_NODE
                        ));
                    }
                    values.iter().take(MAX_CHILDREN_PER_NODE).collect()
                }
                value => vec![value],
            };
            for (index, candidate) in candidates.into_iter().enumerate() {
                let child_path = format!("{}/K[{}]", pending.path, index);
                match resolve_object(doc, candidate) {
                    Ok((object, object_id)) => {
                        if let Some(id) = object_id {
                            if pending.ancestors.contains(&id) || pending.object_id == Some(id) {
                                findings.push(format!(
                                    "Structure cycle at {} ({})",
                                    child_path,
                                    object_id_string(id)
                                ));
                                children.push(empty_node(
                                    format!("cycle:{}:{}@{}", id.0, id.1, child_path),
                                    Some(id),
                                    AccessibilityStructureNodeKind::Unknown,
                                    Some("Cycle traversal stopped".to_string()),
                                ));
                                continue;
                            }
                        }
                        let mut ancestors = pending.ancestors.clone();
                        if let Some(id) = pending.object_id {
                            ancestors.insert(id);
                        }
                        children.push(build_node(
                            doc,
                            PendingNode {
                                object,
                                object_id,
                                path: child_path,
                                depth: pending.depth + 1,
                                ancestors,
                            },
                            budget,
                            page_indices,
                        ));
                    }
                    Err(message) => {
                        let dangling_id = candidate.as_reference().ok();
                        children.push(empty_node(
                            dangling_id
                                .map(|id| format!("dangling:{}:{}@{}", id.0, id.1, child_path))
                                .unwrap_or_else(|| format!("unresolved@{}", child_path)),
                            dangling_id,
                            AccessibilityStructureNodeKind::Unknown,
                            Some(message.clone()),
                        ));
                        findings.push(message);
                    }
                }
            }
        }
    }
    AccessibilityStructureNode {
        id: occurrence_id,
        kind: kind.clone(),
        raw_type: raw_type.clone(),
        mapped_type: raw_type,
        object_id: pending.object_id.map(object_id_string),
        page_index: dictionary
            .get(b"Pg")
            .and_then(Object::as_reference)
            .ok()
            .and_then(|id| page_indices.get(&id).copied()),
        mcid,
        alt_text_state,
        alt_text_preview,
        actual_text_state,
        actual_text_preview,
        language_state,
        language_preview,
        association_status: if matches!(
            kind,
            AccessibilityStructureNodeKind::MarkedContentReference
                | AccessibilityStructureNodeKind::ObjectReference
        ) {
            if findings.is_empty() {
                "partial"
            } else {
                "unresolved"
            }
        } else {
            "not_applicable"
        }
        .to_string(),
        children,
        findings,
        truncated: false,
    }
}

fn catalog(doc: &Document) -> Result<&Dictionary, String> {
    let root = doc
        .trailer
        .get(b"Root")
        .map_err(|_| "PDF catalog root is missing".to_string())?;
    direct_dictionary(doc, root).ok_or_else(|| "PDF catalog is malformed".to_string())
}

pub(crate) fn inspect_structure_tree_from_document(
    doc: &Document,
) -> Result<AccessibilityStructureInspection, String> {
    let catalog = catalog(doc)?;
    let Some(root_object) = catalog.get(b"StructTreeRoot").ok() else {
        return Ok(AccessibilityStructureInspection {
            schema_version: ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION.to_string(),
            inspection_version: "bounded-structure-inspection/1".to_string(),
            root: None,
            findings: vec!["Structure tree root is missing".to_string()],
            warnings: Vec::new(),
            truncated: false,
            limitations: structure_limitations(),
        });
    };
    let (root_object, root_id) = resolve_object(doc, root_object)?;
    let mut budget = TraversalBudget::new();
    let page_indices = doc
        .get_pages()
        .into_iter()
        .filter_map(|(number, id)| number.checked_sub(1).map(|index| (id, index)))
        .collect::<HashMap<_, _>>();
    let mut root = build_node(
        doc,
        PendingNode {
            object: root_object,
            object_id: root_id,
            path: "StructTreeRoot".to_string(),
            depth: 0,
            ancestors: HashSet::new(),
        },
        &mut budget,
        &page_indices,
    );
    root.kind = AccessibilityStructureNodeKind::DocumentRoot;
    let (role_map, role_findings, role_truncated) = role_map_from_root(doc, catalog);
    apply_role_mapping(&mut root, &role_map);
    root.findings.extend(role_findings);
    if role_truncated {
        budget.truncated = true;
        budget
            .warnings
            .push("RoleMap inspection was truncated".to_string());
    }
    let findings = collect_structure_findings(&root);
    Ok(AccessibilityStructureInspection {
        schema_version: ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION.to_string(),
        inspection_version: "bounded-structure-inspection/1".to_string(),
        root: Some(root),
        findings,
        warnings: budget.warnings,
        truncated: budget.truncated,
        limitations: structure_limitations(),
    })
}

fn structure_limitations() -> Vec<String> {
    vec![
        "Full marked-content sequence validation across page and Form XObject content streams is not implemented".to_string(),
        "PDF/UA-2 namespace and MathML semantics are only inventoried, not fully validated".to_string(),
    ]
}

fn collect_structure_findings(root: &AccessibilityStructureNode) -> Vec<String> {
    let mut findings = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        for finding in &node.findings {
            if findings.len() >= MAX_EVIDENCE_PER_CHECK {
                return findings;
            }
            findings.push(format!("{}: {}", node.id, finding));
        }
        stack.extend(node.children.iter());
    }
    findings
}

fn structure_root_dictionary<'a>(
    doc: &'a Document,
    catalog: &'a Dictionary,
) -> Option<&'a Dictionary> {
    catalog
        .get(b"StructTreeRoot")
        .ok()
        .and_then(|object| direct_dictionary(doc, object))
}

fn role_map_from_root(
    doc: &Document,
    catalog: &Dictionary,
) -> (HashMap<String, String>, Vec<String>, bool) {
    let mut result = HashMap::new();
    let mut findings = Vec::new();
    let Some(root) = structure_root_dictionary(doc, catalog) else {
        return (result, findings, false);
    };
    let Some(role_map) = root.get(b"RoleMap").ok() else {
        return (result, findings, false);
    };
    let Some(dictionary) = direct_dictionary(doc, role_map) else {
        findings.push("RoleMap is present but is not a resolvable dictionary".to_string());
        return (result, findings, false);
    };
    let truncated = dictionary.len() > MAX_ROLE_MAP_ENTRIES;
    for (key, value) in dictionary.iter().take(MAX_ROLE_MAP_ENTRIES) {
        let source = bounded_preview(&String::from_utf8_lossy(key));
        match name_value(Some(value)) {
            Some(target) => {
                result.insert(source, target);
            }
            None => findings.push(format!("RoleMap target for /{} is not a name", source)),
        }
    }
    if truncated {
        findings.push(format!(
            "RoleMap exceeds the {} entry limit",
            MAX_ROLE_MAP_ENTRIES
        ));
    }
    (result, findings, truncated)
}

fn is_common_standard_structure_type(value: &str) -> bool {
    matches!(
        value,
        "Document"
            | "Part"
            | "Art"
            | "Sect"
            | "Div"
            | "BlockQuote"
            | "Caption"
            | "TOC"
            | "TOCI"
            | "Index"
            | "NonStruct"
            | "Private"
            | "P"
            | "H"
            | "H1"
            | "H2"
            | "H3"
            | "H4"
            | "H5"
            | "H6"
            | "L"
            | "LI"
            | "Lbl"
            | "LBody"
            | "Table"
            | "TR"
            | "TH"
            | "TD"
            | "THead"
            | "TBody"
            | "TFoot"
            | "Span"
            | "Quote"
            | "Note"
            | "Reference"
            | "BibEntry"
            | "Code"
            | "Link"
            | "Annot"
            | "Ruby"
            | "RB"
            | "RT"
            | "RP"
            | "Warichu"
            | "WT"
            | "WP"
            | "Figure"
            | "Formula"
            | "Form"
    )
}

fn apply_role_mapping(node: &mut AccessibilityStructureNode, role_map: &HashMap<String, String>) {
    if matches!(node.kind, AccessibilityStructureNodeKind::StructureElement) {
        match node.raw_type.clone() {
            None => {
                node.mapped_type = Some("unknown".to_string());
                node.findings.push("StructElem is missing /S".to_string());
            }
            Some(raw) if is_common_standard_structure_type(&raw) => node.mapped_type = Some(raw),
            Some(raw) => {
                let mut current = raw.clone();
                let mut seen = HashSet::new();
                let mut resolved = None;
                while let Some(target) = role_map.get(&current) {
                    if !seen.insert(current.clone()) {
                        node.findings
                            .push(format!("RoleMap cycle while resolving /{}", raw));
                        break;
                    }
                    current = target.clone();
                    if is_common_standard_structure_type(&current) {
                        resolved = Some(current.clone());
                        break;
                    }
                    if seen.len() > MAX_ROLE_MAP_ENTRIES {
                        node.findings
                            .push(format!("RoleMap chain for /{} exceeds the limit", raw));
                        break;
                    }
                }
                if resolved.is_none() {
                    node.findings.push(format!(
                        "Custom structure type /{} is not mapped to a supported standard type",
                        raw
                    ));
                }
                node.mapped_type = Some(resolved.unwrap_or_else(|| "unmapped".to_string()));
            }
        }
    }
    for child in &mut node.children {
        apply_role_mapping(child, role_map);
    }
}

fn role_mapping_findings(root: Option<&AccessibilityStructureNode>) -> Vec<AccessibilityEvidence> {
    let mut result = Vec::new();
    let mut stack = root.into_iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        for finding in &node.findings {
            if finding.contains("RoleMap")
                || finding.contains("structure type")
                || finding.contains("missing /S")
            {
                let mut item = evidence(finding.clone());
                item.object_id = node.object_id.clone();
                item.structure_path = Some(node.id.clone());
                result.push(item);
                if result.len() >= MAX_EVIDENCE_PER_CHECK {
                    return result;
                }
            }
        }
        stack.extend(node.children.iter());
    }
    result
}

struct ParentTreeInspection {
    result: AccessibilityCheckResult,
    evidence: Vec<AccessibilityEvidence>,
    truncated: bool,
}

fn inspect_parent_tree(doc: &Document, catalog: &Dictionary) -> ParentTreeInspection {
    let Some(root) = structure_root_dictionary(doc, catalog) else {
        return ParentTreeInspection {
            result: AccessibilityCheckResult::NotApplicable,
            evidence: Vec::new(),
            truncated: false,
        };
    };
    let Some(parent_tree) = root.get(b"ParentTree").ok() else {
        return ParentTreeInspection {
            result: AccessibilityCheckResult::Fail,
            evidence: vec![evidence("StructTreeRoot ParentTree is missing")],
            truncated: false,
        };
    };
    let mut stack = vec![(parent_tree.clone(), 0usize)];
    let mut visited = HashSet::new();
    let mut keys = HashSet::new();
    let mut findings = Vec::new();
    let mut entries = 0usize;
    let mut truncated = false;
    while let Some((candidate, depth)) = stack.pop() {
        if depth > MAX_NUMBER_TREE_DEPTH {
            findings.push(evidence("ParentTree number-tree depth limit reached"));
            truncated = true;
            break;
        }
        let (object, object_id) = match resolve_object(doc, &candidate) {
            Ok(value) => value,
            Err(message) => {
                findings.push(evidence(message));
                continue;
            }
        };
        if let Some(id) = object_id {
            if !visited.insert(id) {
                findings.push(evidence(format!(
                    "ParentTree cycle or duplicate node at {}",
                    object_id_string(id)
                )));
                continue;
            }
        }
        let Some(dictionary) = direct_dictionary(doc, &object) else {
            findings.push(evidence("ParentTree node is not a dictionary"));
            continue;
        };
        if let Ok(nums) = dictionary.get(b"Nums") {
            let Ok(values) = nums.as_array() else {
                findings.push(evidence("ParentTree Nums is not an array"));
                continue;
            };
            if values.len() % 2 != 0 {
                findings.push(evidence("ParentTree Nums has an odd item count"));
            }
            for pair in values.chunks(2) {
                if pair.len() != 2 {
                    break;
                }
                entries += 1;
                if entries > MAX_PARENT_TREE_ENTRIES {
                    findings.push(evidence(format!(
                        "ParentTree exceeds the {} entry limit",
                        MAX_PARENT_TREE_ENTRIES
                    )));
                    truncated = true;
                    break;
                }
                match pair[0].as_i64() {
                    Ok(key) if key >= 0 => {
                        if !keys.insert(key) {
                            findings
                                .push(evidence(format!("ParentTree key {} is duplicated", key)));
                        }
                    }
                    _ => findings.push(evidence("ParentTree key is not a non-negative integer")),
                }
                match &pair[1] {
                    Object::Reference(id) if doc.get_object(*id).is_err() => {
                        findings.push(evidence(format!(
                            "Dangling ParentTree value {}",
                            object_id_string(*id)
                        )))
                    }
                    Object::Array(items) => {
                        for item in items {
                            if let Object::Reference(id) = item {
                                if doc.get_object(*id).is_err() {
                                    findings.push(evidence(format!(
                                        "Dangling ParentTree array value {}",
                                        object_id_string(*id)
                                    )));
                                }
                            } else if !matches!(item, Object::Null) {
                                findings.push(evidence(
                                    "ParentTree array value is neither a reference nor null",
                                ));
                            }
                        }
                    }
                    Object::Reference(_) | Object::Null => {}
                    _ => findings.push(evidence("ParentTree value has an unsupported type")),
                }
                if findings.len() >= MAX_EVIDENCE_PER_CHECK || truncated {
                    break;
                }
            }
        }
        if let Ok(kids) = dictionary.get(b"Kids") {
            match kids.as_array() {
                Ok(values) => {
                    if values.len() > MAX_CHILDREN_PER_NODE {
                        findings.push(evidence(format!(
                            "ParentTree child count {} exceeds limit {}",
                            values.len(),
                            MAX_CHILDREN_PER_NODE
                        )));
                        truncated = true;
                    }
                    stack.extend(
                        values
                            .iter()
                            .take(MAX_CHILDREN_PER_NODE)
                            .cloned()
                            .map(|value| (value, depth + 1)),
                    );
                }
                Err(_) => findings.push(evidence("ParentTree Kids is not an array")),
            }
        }
        if findings.len() >= MAX_EVIDENCE_PER_CHECK || truncated {
            break;
        }
    }

    for (page_number, page_id) in doc.get_pages() {
        let Ok(page) = doc.get_dictionary(page_id) else {
            continue;
        };
        if let Ok(value) = page.get(b"StructParents") {
            match value.as_i64() {
                Ok(key) if key >= 0 && keys.contains(&key) => {}
                Ok(key) if key >= 0 => {
                    let mut item = evidence(format!(
                        "Page StructParents key {} has no ParentTree entry",
                        key
                    ));
                    item.page_index = page_number.checked_sub(1);
                    findings.push(item);
                }
                _ => findings.push(evidence("Page StructParents is not a non-negative integer")),
            }
        }
        if let Ok(annots) = page.get(b"Annots") {
            let annotation_objects = match annots {
                Object::Array(values) => Some(values.as_slice()),
                Object::Reference(id) => doc
                    .get_object(*id)
                    .ok()
                    .and_then(|value| value.as_array().ok())
                    .map(Vec::as_slice),
                _ => None,
            };
            if let Some(annotation_objects) = annotation_objects {
                for annotation in annotation_objects {
                    let Some(dictionary) = direct_dictionary(doc, annotation) else {
                        continue;
                    };
                    if let Ok(value) = dictionary.get(b"StructParent") {
                        match value.as_i64() {
                            Ok(key) if key >= 0 && keys.contains(&key) => {}
                            Ok(key) if key >= 0 => {
                                let mut item = evidence(format!(
                                    "Annotation StructParent key {} has no ParentTree entry",
                                    key
                                ));
                                item.page_index = page_number.checked_sub(1);
                                findings.push(item);
                            }
                            _ => findings.push(evidence(
                                "Annotation StructParent is not a non-negative integer",
                            )),
                        }
                    }
                    if findings.len() >= MAX_EVIDENCE_PER_CHECK {
                        break;
                    }
                }
            }
        }
        if findings.len() >= MAX_EVIDENCE_PER_CHECK {
            break;
        }
    }
    ParentTreeInspection {
        result: if truncated {
            AccessibilityCheckResult::Indeterminate
        } else if findings.is_empty() {
            AccessibilityCheckResult::Pass
        } else {
            AccessibilityCheckResult::Fail
        },
        evidence: findings,
        truncated,
    }
}

fn xmp_facts(doc: &Document, catalog: &Dictionary) -> XmpFacts {
    let Some(metadata) = catalog.get(b"Metadata").ok() else {
        return XmpFacts::default();
    };
    let Some(stream) = (match metadata {
        Object::Reference(id) => doc
            .get_object(*id)
            .ok()
            .and_then(|value| value.as_stream().ok()),
        value => value.as_stream().ok(),
    }) else {
        return XmpFacts {
            state: AccessibilityTextState::Invalid,
            warning: Some("Catalog Metadata is not a stream".to_string()),
            ..XmpFacts::default()
        };
    };
    if stream.content.len() > MAX_XMP_BYTES {
        return XmpFacts {
            state: AccessibilityTextState::Invalid,
            warning: Some("Compressed XMP exceeds the audit limit".to_string()),
            ..XmpFacts::default()
        };
    }
    let decoded = if stream.dict.has(b"Filter") {
        stream.decompressed_content()
    } else {
        Ok(stream.content.clone())
    };
    let bytes = match decoded {
        Ok(bytes) if bytes.len() <= MAX_XMP_BYTES => bytes,
        Ok(_) => {
            return XmpFacts {
                state: AccessibilityTextState::Invalid,
                warning: Some("Decoded XMP exceeds the audit limit".to_string()),
                ..XmpFacts::default()
            }
        }
        Err(_) => {
            return XmpFacts {
                state: AccessibilityTextState::Invalid,
                warning: Some("XMP stream could not be decoded".to_string()),
                ..XmpFacts::default()
            }
        }
    };
    let Ok(xml) = std::str::from_utf8(&bytes) else {
        return XmpFacts {
            state: AccessibilityTextState::Invalid,
            warning: Some("XMP is not valid UTF-8 XML".to_string()),
            ..XmpFacts::default()
        };
    };
    parse_xmp(xml)
}

fn parse_xmp(xml: &str) -> XmpFacts {
    const PDFUA_NS: &[u8] = b"http://www.aiim.org/pdfua/ns/id/";
    const DC_NS: &[u8] = b"http://purl.org/dc/elements/1.1/";
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut facts = XmpFacts::default();
    let mut current: Option<&'static str> = None;
    let mut in_title = 0usize;
    let mut buffer = Vec::new();
    loop {
        match reader.read_resolved_event_into(&mut buffer) {
            Ok((ResolveResult::Bound(namespace), Event::Start(element))) => {
                let local = element.local_name();
                current = if namespace.as_ref() == PDFUA_NS && local.as_ref() == b"part" {
                    Some("part")
                } else if namespace.as_ref() == PDFUA_NS && local.as_ref() == b"rev" {
                    Some("rev")
                } else {
                    None
                };
                if namespace.as_ref() == DC_NS && local.as_ref() == b"title" {
                    in_title = 1;
                } else if in_title > 0 {
                    in_title += 1;
                }
            }
            Ok((_, Event::End(_))) => {
                current = None;
                in_title = in_title.saturating_sub(1);
            }
            Ok((_, Event::Text(text))) => {
                let value = text
                    .decode()
                    .ok()
                    .and_then(|decoded| {
                        quick_xml::escape::unescape(&decoded)
                            .ok()
                            .map(|value| value.into_owned())
                    })
                    .map(|value| bounded_preview(value.trim()))
                    .filter(|value| !value.is_empty());
                if let Some(value) = value {
                    match current {
                        Some("part") => facts.part = Some(value),
                        Some("rev") => facts.revision = Some(value),
                        _ if in_title > 0 && facts.title.is_none() => facts.title = Some(value),
                        _ => {}
                    }
                }
            }
            Ok((_, Event::Eof)) => break,
            Ok(_) => {}
            Err(_) => {
                facts.state = AccessibilityTextState::Invalid;
                facts.warning = Some("XMP XML is malformed".to_string());
                return facts;
            }
        }
        buffer.clear();
    }
    facts.state = if facts.part.is_some() || facts.revision.is_some() || facts.title.is_some() {
        AccessibilityTextState::Present
    } else {
        AccessibilityTextState::Empty
    };
    facts
}

fn standard(standard: &str, clause: &str) -> AccessibilityStandardReference {
    AccessibilityStandardReference {
        standard: standard.to_string(),
        clause: clause.to_string(),
        reference_type: "derived_requirement".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    id: &str,
    title: &str,
    category: &str,
    result: AccessibilityCheckResult,
    severity: AccessibilitySeverity,
    automation: AccessibilityAutomation,
    evidence: Vec<AccessibilityEvidence>,
    references: Vec<AccessibilityStandardReference>,
    notes: Vec<String>,
) -> AccessibilityCheck {
    AccessibilityCheck {
        id: id.to_string(),
        title: title.to_string(),
        category: category.to_string(),
        result,
        severity,
        automation,
        evidence: evidence.into_iter().take(MAX_EVIDENCE_PER_CHECK).collect(),
        standard_references: references,
        conformance_claim: false,
        notes,
    }
}

fn evidence(message: impl Into<String>) -> AccessibilityEvidence {
    AccessibilityEvidence {
        object_id: None,
        page_index: None,
        structure_path: None,
        field: None,
        message: message.into(),
        value_preview: None,
    }
}

fn has_nonempty_text(state: AccessibilityTextState) -> bool {
    state == AccessibilityTextState::Present
}

fn effective_version(header: Option<&str>, catalog: Option<&str>) -> Option<String> {
    fn parts(value: &str) -> Option<(u32, u32)> {
        let mut values = value.split('.');
        Some((values.next()?.parse().ok()?, values.next()?.parse().ok()?))
    }
    match (header, catalog) {
        (Some(header), Some(catalog)) => match (parts(header), parts(catalog)) {
            (Some(a), Some(b)) => Some(if b > a { catalog } else { header }.to_string()),
            _ => None,
        },
        (Some(value), None) | (None, Some(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn select_profile(
    version: Option<&str>,
    declaration: &PdfUaDeclaration,
) -> AccessibilityReferenceProfile {
    match (
        declaration.part.as_deref(),
        declaration.revision.as_deref(),
        version,
    ) {
        (Some("1"), _, Some("1.7")) => AccessibilityReferenceProfile::PdfUa1Reference,
        (Some("2"), Some("2024"), Some("2.0")) => AccessibilityReferenceProfile::PdfUa2Reference,
        (Some(_), _, _) => AccessibilityReferenceProfile::Undetermined,
        (None, _, Some("1.7")) => AccessibilityReferenceProfile::PdfUa1Reference,
        (None, _, Some("2.0")) => AccessibilityReferenceProfile::PdfUa2Reference,
        (None, _, Some(_)) => AccessibilityReferenceProfile::UnsupportedPdfVersion,
        _ => AccessibilityReferenceProfile::Undetermined,
    }
}

fn count_figure_states(root: Option<&AccessibilityStructureNode>) -> (usize, usize) {
    let mut figures = 0;
    let mut invalid = 0;
    let mut stack = root.into_iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        if node.raw_type.as_deref() == Some("Figure") {
            figures += 1;
            if !has_nonempty_text(node.alt_text_state) {
                invalid += 1;
            }
        }
        stack.extend(node.children.iter());
    }
    (figures, invalid)
}

fn valid_language_tag(value: Option<&str>) -> bool {
    let Some(value) = value else { return false };
    let mut parts = value.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    (2..=8).contains(&primary.len())
        && primary.bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| {
            !part.is_empty()
                && part.len() <= 8
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

pub(crate) fn audit_accessibility_document(doc: &Document) -> AccessibilityAuditReport {
    let inspection = inspect_structure_tree_from_document(doc).unwrap_or_else(|message| {
        AccessibilityStructureInspection {
            schema_version: ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION.to_string(),
            inspection_version: "bounded-structure-inspection/1".to_string(),
            root: None,
            findings: vec![message],
            warnings: Vec::new(),
            truncated: false,
            limitations: structure_limitations(),
        }
    });
    let catalog = catalog(doc).ok();
    let xmp = catalog
        .map(|value| xmp_facts(doc, value))
        .unwrap_or_default();
    let header_version = Some(doc.version.clone());
    let catalog_version = catalog.and_then(|value| name_value(value.get(b"Version").ok()));
    let effective_version =
        effective_version(header_version.as_deref(), catalog_version.as_deref());
    let declaration = PdfUaDeclaration {
        part: xmp.part.clone(),
        revision: xmp.revision.clone(),
        state: if xmp.part.is_some() {
            AccessibilityTextState::Present
        } else if xmp.state == AccessibilityTextState::Invalid {
            AccessibilityTextState::Invalid
        } else {
            AccessibilityTextState::Missing
        },
    };
    let profile = select_profile(effective_version.as_deref(), &declaration);
    let mark_info = catalog
        .and_then(|value| value.get_deref(b"MarkInfo", doc).ok())
        .and_then(|value| value.as_dict().ok());
    let has_mark_info = mark_info.is_some();
    let has_marked_entry = mark_info.is_some_and(|value| value.get(b"Marked").is_ok());
    let marked = mark_info
        .and_then(|value| value.get(b"Marked").ok())
        .and_then(|value| value.as_bool().ok())
        .unwrap_or(false);
    let display_title = catalog
        .and_then(|value| value.get_deref(b"ViewerPreferences", doc).ok())
        .and_then(|value| value.as_dict().ok())
        .and_then(|value| value.get(b"DisplayDocTitle").ok())
        .and_then(|value| value.as_bool().ok())
        .unwrap_or(false);
    let (language_state, language_preview) = catalog
        .map(|value| text_value(value.get(b"Lang").ok()))
        .unwrap_or((AccessibilityTextState::Missing, None));
    let (info_title, info_title_preview) = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|value| direct_dictionary(doc, value))
        .map(|value| text_value(value.get(b"Title").ok()))
        .unwrap_or((AccessibilityTextState::Missing, None));
    let xmp_title = if xmp.state == AccessibilityTextState::Invalid {
        AccessibilityTextState::Invalid
    } else if xmp.title.is_some() {
        AccessibilityTextState::Present
    } else {
        AccessibilityTextState::Missing
    };
    let facts = AccessibilityDocumentFacts {
        header_version,
        catalog_version,
        effective_version,
        pdfua_declaration: declaration.clone(),
        page_count: doc.get_pages().len().try_into().unwrap_or(u32::MAX),
        encrypted: doc.is_encrypted(),
        has_structure_tree: inspection.root.is_some(),
        has_mark_info,
        has_marked_entry,
        marked,
        display_document_title: display_title,
        document_language: language_state,
        xmp_title,
        info_title,
    };
    let profile_reference = match profile {
        AccessibilityReferenceProfile::PdfUa1Reference => {
            vec![standard("ISO 14289-1:2014", "5, 6.1")]
        }
        AccessibilityReferenceProfile::PdfUa2Reference => {
            vec![standard("ISO 14289-2:2024", "5, 6.1")]
        }
        _ => Vec::new(),
    };
    let mut checks = vec![
        check(
            "document.pdf-version",
            "PDF version and audit reference",
            "document",
            if matches!(
                profile,
                AccessibilityReferenceProfile::Undetermined
                    | AccessibilityReferenceProfile::UnsupportedPdfVersion
            ) {
                AccessibilityCheckResult::Indeterminate
            } else {
                AccessibilityCheckResult::Pass
            },
            AccessibilitySeverity::Major,
            AccessibilityAutomation::Machine,
            vec![evidence(format!(
                "Effective PDF version: {}",
                facts.effective_version.as_deref().unwrap_or("unknown")
            ))],
            profile_reference,
            Vec::new(),
        ),
        check(
            "document.pdfua-declaration",
            "PDF/UA XMP declaration",
            "metadata",
            if declaration.state == AccessibilityTextState::Present
                && !matches!(profile, AccessibilityReferenceProfile::Undetermined)
            {
                AccessibilityCheckResult::Pass
            } else if declaration.state == AccessibilityTextState::Invalid {
                AccessibilityCheckResult::Indeterminate
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Major,
            AccessibilityAutomation::Machine,
            vec![evidence(
                declaration
                    .part
                    .as_deref()
                    .map(|part| format!("PDF/UA part {} declaration found", part))
                    .unwrap_or_else(|| "PDF/UA declaration is missing or unreadable".to_string()),
            )],
            vec![standard("ISO 14289-1:2014 / ISO 14289-2:2024", "Clause 5")],
            vec!["A declaration is not proof of conformance".to_string()],
        ),
        check(
            "tagging.structure-tree-root",
            "Structure tree root",
            "structure",
            if facts.has_structure_tree {
                AccessibilityCheckResult::Pass
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Critical,
            AccessibilityAutomation::Machine,
            vec![evidence(if facts.has_structure_tree {
                "StructTreeRoot is present"
            } else {
                "StructTreeRoot is missing"
            })],
            vec![
                standard("ISO 14289-1:2014", "7.1"),
                standard("ISO 14289-2:2024", "8.2.1"),
            ],
            Vec::new(),
        ),
        check(
            "tagging.mark-info",
            "Marked document flag",
            "structure",
            if facts.has_structure_tree && facts.marked {
                AccessibilityCheckResult::Pass
            } else if facts.has_structure_tree
                || facts.has_mark_info
                || facts.has_marked_entry
                || facts.marked
            {
                AccessibilityCheckResult::Warning
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Major,
            AccessibilityAutomation::Machine,
            vec![evidence(format!(
                "MarkInfo dictionary present: {}; Marked entry present: {}; Marked is true: {}; StructTreeRoot readable: {}",
                facts.has_mark_info,
                facts.has_marked_entry,
                facts.marked,
                facts.has_structure_tree
            ))],
            vec![
                standard("ISO 14289-1:2014", "6.2"),
                standard("ISO 14289-2:2024", "6.2"),
            ],
            Vec::new(),
        ),
        check(
            "document.language",
            "Document language",
            "language",
            if has_nonempty_text(facts.document_language)
                && valid_language_tag(language_preview.as_deref())
            {
                AccessibilityCheckResult::Pass
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Major,
            AccessibilityAutomation::Mixed,
            vec![evidence(format!(
                "Catalog Lang state: {:?}",
                facts.document_language
            ))],
            vec![
                standard("ISO 14289-1:2014", "7.2"),
                standard("ISO 14289-2:2024", "8.4.4"),
            ],
            vec!["Language correctness requires human review".to_string()],
        ),
        check(
            "metadata.document-title",
            "XMP document title",
            "metadata",
            if has_nonempty_text(facts.xmp_title) {
                if info_title_preview.as_deref().is_some_and(|info| {
                    xmp.title.as_deref().is_some_and(|xmp_title| {
                        !info.trim().eq_ignore_ascii_case(xmp_title.trim())
                    })
                }) {
                    AccessibilityCheckResult::Warning
                } else {
                    AccessibilityCheckResult::Pass
                }
            } else if facts.xmp_title == AccessibilityTextState::Invalid {
                AccessibilityCheckResult::Indeterminate
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Major,
            AccessibilityAutomation::Machine,
            vec![evidence(format!(
                "XMP dc:title state: {:?}; Info Title state: {:?}",
                facts.xmp_title, facts.info_title
            ))],
            vec![
                standard("ISO 14289-1:2014", "7.1"),
                standard("ISO 14289-2:2024", "8.11.1"),
            ],
            vec![
                "Info dictionary Title is supplemental evidence and cannot satisfy this check"
                    .to_string(),
            ],
        ),
        check(
            "metadata.display-document-title",
            "Display document title preference",
            "metadata",
            if display_title {
                AccessibilityCheckResult::Pass
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Minor,
            AccessibilityAutomation::Machine,
            vec![evidence(if display_title {
                "DisplayDocTitle is true"
            } else {
                "DisplayDocTitle is absent or false"
            })],
            vec![
                standard("ISO 14289-1:2014", "7.1"),
                standard("ISO 14289-2:2024", "8.11.2"),
            ],
            Vec::new(),
        ),
        check(
            "structure.integrity",
            "Bounded structure integrity",
            "structure",
            if inspection.truncated {
                AccessibilityCheckResult::Indeterminate
            } else if inspection.findings.is_empty() {
                AccessibilityCheckResult::Pass
            } else {
                AccessibilityCheckResult::Fail
            },
            AccessibilitySeverity::Critical,
            AccessibilityAutomation::Machine,
            inspection
                .findings
                .iter()
                .take(MAX_EVIDENCE_PER_CHECK)
                .map(|message| evidence(message.clone()))
                .collect(),
            vec![standard("ISO 32000-1 / ISO 32000-2", "Logical structure")],
            structure_limitations(),
        ),
    ];
    let role_evidence = role_mapping_findings(inspection.root.as_ref());
    checks.push(check(
        "structure.role-mapping",
        "Role mapping and namespaces",
        "structure",
        if !facts.has_structure_tree {
            AccessibilityCheckResult::NotApplicable
        } else if role_evidence.is_empty()
            && profile == AccessibilityReferenceProfile::PdfUa1Reference
        {
            AccessibilityCheckResult::Pass
        } else if role_evidence.is_empty() {
            AccessibilityCheckResult::Indeterminate
        } else {
            AccessibilityCheckResult::Fail
        },
        AccessibilitySeverity::Major,
        AccessibilityAutomation::Mixed,
        role_evidence,
        vec![
            standard("ISO 14289-1:2014", "7.1"),
            standard("ISO 14289-2:2024", "8.2.4"),
        ],
        vec!["PDF/UA-2 namespace semantics are not fully implemented".to_string()],
    ));
    let parent_tree = catalog
        .map(|value| inspect_parent_tree(doc, value))
        .unwrap_or(ParentTreeInspection {
            result: AccessibilityCheckResult::Indeterminate,
            evidence: vec![evidence("Catalog could not be read")],
            truncated: false,
        });
    let parent_tree_truncated = parent_tree.truncated;
    checks.push(check("structure.parent-tree", "Parent tree associations", "structure", parent_tree.result,
        AccessibilitySeverity::Major, AccessibilityAutomation::Machine, parent_tree.evidence,
        vec![standard("ISO 32000-1 / ISO 32000-2", "Parent tree and structure associations")],
        vec!["This check validates the number-tree and declared StructParent(s) lookups, not full content-stream MCID sequences".to_string()]));
    let (figures, invalid_figures) = count_figure_states(inspection.root.as_ref());
    checks.push(check(
        "structure.figure-alternative-description",
        "Figure alternative descriptions",
        "structure",
        if figures == 0 {
            AccessibilityCheckResult::NotApplicable
        } else if invalid_figures == 0 {
            AccessibilityCheckResult::Pass
        } else {
            AccessibilityCheckResult::Fail
        },
        AccessibilitySeverity::Major,
        AccessibilityAutomation::Mixed,
        vec![evidence(format!(
            "Figures inspected: {}; missing, empty, whitespace-only or invalid alternatives: {}",
            figures, invalid_figures
        ))],
        vec![
            standard("ISO 14289-1:2014", "7.3"),
            standard("Matterhorn Protocol 1.1", "13-004"),
        ],
        vec!["Alternative-description quality and context require human review".to_string()],
    ));
    for (id, title, category) in [
        (
            "structure.content-association",
            "Marked-content associations",
            "structure",
        ),
        (
            "structure.heading-consistency",
            "Heading structure",
            "structure",
        ),
        ("structure.list-consistency", "List structure", "structure"),
        (
            "structure.table-consistency",
            "Table structure",
            "structure",
        ),
        (
            "annotations.structure-association",
            "Annotation structure associations",
            "annotations",
        ),
        (
            "annotations.inventory",
            "Annotation inventory",
            "annotations",
        ),
        (
            "annotations.tab-order",
            "Page annotation tab order",
            "annotations",
        ),
        (
            "annotations.alternative-description",
            "Annotation alternative descriptions",
            "annotations",
        ),
        ("forms.inventory", "Interactive form inventory", "forms"),
        ("forms.accessible-name", "Form accessible names", "forms"),
        (
            "forms.widget-association",
            "Form widget structure associations",
            "forms",
        ),
        ("text.availability", "Extractable text availability", "text"),
        ("text.unicode-mapping", "Unicode character mapping", "text"),
        (
            "security.accessibility-permission",
            "Accessibility extraction permission",
            "encryption",
        ),
    ] {
        checks.push(check(
            id,
            title,
            category,
            AccessibilityCheckResult::NotChecked,
            AccessibilitySeverity::Info,
            AccessibilityAutomation::Machine,
            Vec::new(),
            Vec::new(),
            vec![
                "This control is explicitly outside the implemented machine-check coverage"
                    .to_string(),
            ],
        ));
    }
    let not_implemented_check_ids = checks
        .iter()
        .filter(|item| item.result == AccessibilityCheckResult::NotChecked)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let implemented_check_ids = checks
        .iter()
        .filter(|item| item.result != AccessibilityCheckResult::NotChecked)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let mut incomplete_check_ids = Vec::new();
    if inspection.truncated {
        incomplete_check_ids.push("structure.integrity".to_string());
    }
    if parent_tree_truncated {
        incomplete_check_ids.push("structure.parent-tree".to_string());
    }
    if doc.objects.len() > MAX_PDF_OBJECTS {
        incomplete_check_ids.push("document.pdf-version".to_string());
    }
    if facts.xmp_title == AccessibilityTextState::Invalid {
        incomplete_check_ids.push("metadata.document-title".to_string());
    }
    if matches!(
        profile,
        AccessibilityReferenceProfile::PdfUa2Reference
            | AccessibilityReferenceProfile::Undetermined
            | AccessibilityReferenceProfile::UnsupportedPdfVersion
    ) {
        incomplete_check_ids.push("document.pdf-version".to_string());
    }
    let incomplete = !incomplete_check_ids.is_empty();
    let findings = checks.iter().any(|item| {
        matches!(
            item.result,
            AccessibilityCheckResult::Fail | AccessibilityCheckResult::Warning
        )
    });
    let summary_status = if incomplete {
        AccessibilitySummaryStatus::AuditIncomplete
    } else if findings {
        AccessibilitySummaryStatus::FindingsDetected
    } else {
        AccessibilitySummaryStatus::NoFindingsInImplementedChecks
    };
    let mut warnings = inspection.warnings.clone();
    if doc.objects.len() > MAX_PDF_OBJECTS {
        warnings.push(format!(
            "PDF object count exceeds the {} audit limit",
            MAX_PDF_OBJECTS
        ));
    }
    if let Some(warning) = xmp.warning {
        warnings.push(warning);
    }
    warnings.push(
        "This basic audit does not establish PDF/UA conformance or document accessibility"
            .to_string(),
    );
    let human_review = human_review_items();
    let mut report = AccessibilityAuditReport {
        schema_version: ACCESSIBILITY_AUDIT_SCHEMA_VERSION.to_string(),
        audit_version: ACCESSIBILITY_AUDIT_VERSION.to_string(),
        reference_profile: profile,
        summary_status,
        document_facts: facts,
        checks,
        coverage: AccessibilityCoverage {
            machine_check_count: u32::try_from(implemented_check_ids.len()).unwrap_or(u32::MAX),
            human_review_count: u32::try_from(human_review.len()).unwrap_or(u32::MAX),
            implemented_check_ids,
            not_implemented_check_ids,
            incomplete_check_ids,
            categories: vec![
                AccessibilityCategoryCoverage {
                    category: "document".to_string(),
                    status: "partial".to_string(),
                    limitations: Vec::new(),
                },
                AccessibilityCategoryCoverage {
                    category: "structure".to_string(),
                    status: "partial".to_string(),
                    limitations: structure_limitations(),
                },
                AccessibilityCategoryCoverage {
                    category: "annotations".to_string(),
                    status: "not_checked".to_string(),
                    limitations: vec![
                        "Detailed annotation semantics are not implemented".to_string()
                    ],
                },
                AccessibilityCategoryCoverage {
                    category: "forms".to_string(),
                    status: "not_checked".to_string(),
                    limitations: vec!["Detailed form semantics are not implemented".to_string()],
                },
                AccessibilityCategoryCoverage {
                    category: "text".to_string(),
                    status: "not_checked".to_string(),
                    limitations: vec![
                        "Text availability and Unicode mapping are not implemented".to_string()
                    ],
                },
                AccessibilityCategoryCoverage {
                    category: "encryption".to_string(),
                    status: "not_checked".to_string(),
                    limitations: vec![
                        "Accessibility extraction permission is not implemented".to_string()
                    ],
                },
            ],
            limitations: vec![
                "Machine-verifiable checks cover only a subset of PDF/UA requirements".to_string(),
                "Human review is required".to_string(),
            ],
        },
        human_review,
        warnings,
        truncated: inspection.truncated || parent_tree_truncated,
    };
    enforce_report_size(&mut report, MAX_REPORT_BYTES);
    report
}

fn enforce_report_size(report: &mut AccessibilityAuditReport, max_bytes: usize) {
    if serde_json::to_vec(&*report)
        .map(|value| value.len())
        .unwrap_or(usize::MAX)
        > max_bytes
    {
        for item in &mut report.checks {
            item.evidence.clear();
        }
        report.warnings.truncate(32);
        report.warnings.push(
            "Audit evidence was removed because the serialized report limit was reached"
                .to_string(),
        );
        report.truncated = true;
        report.summary_status = AccessibilitySummaryStatus::AuditIncomplete;
    }
}

fn human_review_items() -> Vec<AccessibilityHumanReviewItem> {
    [
        (
            "human.logical-reading-order",
            "Logical reading order",
            "reading_order",
        ),
        (
            "human.structure-semantic-accuracy",
            "Semantic accuracy of structure tags",
            "structure",
        ),
        (
            "human.figure-alt-quality",
            "Quality and context of figure alternatives",
            "alternatives",
        ),
        (
            "human.decorative-artifact-use",
            "Decorative content and artifact use",
            "structure",
        ),
        (
            "human.heading-hierarchy",
            "Meaningful heading hierarchy",
            "structure",
        ),
        (
            "human.table-header-associations",
            "Table header associations",
            "tables",
        ),
        ("human.link-purpose", "Link purpose", "annotations"),
        (
            "human.language-accuracy",
            "Document language and language changes",
            "language",
        ),
        (
            "human.color-contrast",
            "Color contrast and color-only information",
            "visual",
        ),
        (
            "human.form-interaction",
            "Form instructions, errors and keyboard order",
            "forms",
        ),
        ("human.ocr-accuracy", "OCR accuracy", "text"),
        (
            "human.math-media-cognitive",
            "Math, media and cognitive accessibility",
            "content",
        ),
    ]
    .into_iter()
    .map(|(id, title, category)| AccessibilityHumanReviewItem {
        id: id.to_string(),
        title: title.to_string(),
        category: category.to_string(),
        result: AccessibilityCheckResult::HumanReviewRequired,
        applies: true,
        notes: vec!["This item cannot be resolved by this machine audit".to_string()],
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream, StringFormat};

    fn minimal_document() -> Document {
        Document::load_mem(include_bytes!("../tests/fixtures/pdf/minimal.pdf"))
            .expect("minimal PDF fixture")
    }

    fn document_with_structure(root: Object, marked: bool) -> Document {
        let mut doc = Document::with_version("1.7");
        let structure_id = doc.add_object(root);
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "StructTreeRoot" => structure_id,
            "MarkInfo" => dictionary! { "Marked" => marked },
            "Lang" => Object::String(b"en-US".to_vec(), StringFormat::Literal),
        });
        doc.trailer.set("Root", catalog_id);
        doc
    }

    fn catalog_mut(doc: &mut Document) -> &mut Dictionary {
        let catalog_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_object_mut(catalog_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
    }

    fn full_implemented_checks_document() -> Document {
        let mut doc = document_with_structure(
            Object::Dictionary(dictionary! {
                "Type" => "StructTreeRoot",
                "ParentTree" => dictionary! { "Nums" => Vec::<Object>::new() },
            }),
            true,
        );
        let metadata = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            br#"<x:xmpmeta xmlns:x='adobe:ns:meta/' xmlns:pdfuaid='http://www.aiim.org/pdfua/ns/id/' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:RDF><rdf:Description><pdfuaid:part>1</pdfuaid:part><dc:title><rdf:Alt><rdf:li xml:lang='x-default'>Accessible title</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#.to_vec(),
        ));
        {
            let catalog = catalog_mut(&mut doc);
            catalog.set("Metadata", metadata);
            catalog.set(
                "ViewerPreferences",
                dictionary! { "DisplayDocTitle" => true },
            );
        }
        let facts = xmp_facts(&doc, catalog(&doc).unwrap());
        assert_eq!(facts.state, AccessibilityTextState::Present, "{facts:?}");
        doc
    }

    fn result_for<'a>(report: &'a AccessibilityAuditReport, id: &str) -> &'a AccessibilityCheck {
        report.checks.iter().find(|check| check.id == id).unwrap()
    }

    fn metadata_stream_mut(doc: &mut Document) -> &mut Stream {
        let metadata_id = catalog(doc)
            .unwrap()
            .get(b"Metadata")
            .unwrap()
            .as_reference()
            .unwrap();
        doc.get_object_mut(metadata_id)
            .unwrap()
            .as_stream_mut()
            .unwrap()
    }

    #[test]
    fn untagged_document_returns_a_bounded_inspection_instead_of_an_error() {
        let inspection = inspect_structure_tree_from_document(&minimal_document()).unwrap();
        assert!(inspection.root.is_none());
        assert!(!inspection.findings.is_empty());
        assert!(!inspection.truncated);
    }

    #[test]
    fn audit_never_returns_a_score_or_a_conformance_claim() {
        let report = audit_accessibility_document(&minimal_document());
        assert_eq!(report.schema_version, ACCESSIBILITY_AUDIT_SCHEMA_VERSION);
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );
        assert!(report.checks.iter().all(|check| !check.conformance_claim));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("score"));
        assert!(!json.contains("uygun görünüyor"));
    }

    #[test]
    fn xmp_parser_requires_the_pdfua_namespace() {
        let facts = parse_xmp(r#"<x xmlns:fake='urn:fake'><fake:part>1</fake:part></x>"#);
        assert!(facts.part.is_none());
    }

    #[test]
    fn xmp_parser_reads_pdfua_identity_and_dc_title() {
        let facts = parse_xmp(
            r#"<x:xmpmeta xmlns:x='adobe:ns:meta/' xmlns:pdfuaid='http://www.aiim.org/pdfua/ns/id/' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:RDF><rdf:Description><pdfuaid:part>2</pdfuaid:part><pdfuaid:rev>2024</pdfuaid:rev><dc:title><rdf:Alt><rdf:li xml:lang='x-default'>Example</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#,
        );
        assert_eq!(facts.part.as_deref(), Some("2"));
        assert_eq!(facts.revision.as_deref(), Some("2024"));
        assert_eq!(facts.title.as_deref(), Some("Example"));
    }

    #[test]
    fn mixed_structure_children_are_preserved_with_unique_ids() {
        let child = dictionary! { "Type" => "StructElem", "S" => "P" };
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "K" => vec![
                Object::Integer(7),
                Object::Dictionary(child.clone()),
                Object::Dictionary(child),
                Object::Dictionary(dictionary! { "Type" => "MCR", "MCID" => 9 }),
                Object::Dictionary(dictionary! { "Type" => "OBJR", "Obj" => Object::Reference((999, 0)) }),
            ],
        };
        let inspection = inspect_structure_tree_from_document(&document_with_structure(
            Object::Dictionary(root),
            true,
        ))
        .unwrap();
        let children = &inspection.root.unwrap().children;
        assert_eq!(children.len(), 5);
        assert_eq!(children[0].kind, AccessibilityStructureNodeKind::Mcid);
        assert_eq!(
            children[3].kind,
            AccessibilityStructureNodeKind::MarkedContentReference
        );
        assert_eq!(children[4].association_status, "unresolved");
        assert_ne!(children[1].id, children[2].id);
    }

    #[test]
    fn self_cycle_is_reported_without_unbounded_traversal() {
        let mut doc = Document::with_version("1.7");
        let element_id = doc.new_object_id();
        doc.objects.insert(
            element_id,
            Object::Dictionary(dictionary! {
                "Type" => "StructElem", "S" => "P", "K" => element_id,
            }),
        );
        let structure_id =
            doc.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => element_id });
        let catalog_id =
            doc.add_object(dictionary! { "Type" => "Catalog", "StructTreeRoot" => structure_id });
        doc.trailer.set("Root", catalog_id);
        let inspection = inspect_structure_tree_from_document(&doc).unwrap();
        assert!(inspection
            .findings
            .iter()
            .any(|finding| finding.contains("cycle")));
    }

    #[test]
    fn role_map_resolves_custom_types_and_reports_cycles() {
        let valid = dictionary! {
            "Type" => "StructTreeRoot",
            "RoleMap" => dictionary! { "CustomHeading" => "H1" },
            "K" => Object::Dictionary(dictionary! { "Type" => "StructElem", "S" => "CustomHeading" }),
        };
        let inspection = inspect_structure_tree_from_document(&document_with_structure(
            Object::Dictionary(valid),
            true,
        ))
        .unwrap();
        assert_eq!(
            inspection.root.unwrap().children[0].mapped_type.as_deref(),
            Some("H1")
        );

        let cyclic = dictionary! {
            "Type" => "StructTreeRoot",
            "RoleMap" => dictionary! { "A" => "B", "B" => "A" },
            "K" => Object::Dictionary(dictionary! { "Type" => "StructElem", "S" => "A" }),
        };
        let inspection = inspect_structure_tree_from_document(&document_with_structure(
            Object::Dictionary(cyclic),
            true,
        ))
        .unwrap();
        assert!(inspection
            .findings
            .iter()
            .any(|finding| finding.contains("RoleMap cycle")));
    }

    #[test]
    fn actual_text_does_not_satisfy_the_figure_alt_check() {
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "K" => Object::Dictionary(dictionary! {
                "Type" => "StructElem", "S" => "Figure",
                "ActualText" => Object::String(b"replacement".to_vec(), StringFormat::Literal),
            }),
        };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        let figure = report
            .checks
            .iter()
            .find(|check| check.id == "structure.figure-alternative-description")
            .unwrap();
        assert_eq!(figure.result, AccessibilityCheckResult::Fail);
    }

    #[test]
    fn parent_tree_requires_entries_for_declared_page_keys() {
        let mut doc = document_with_structure(
            Object::Dictionary(dictionary! {
                "Type" => "StructTreeRoot",
                "ParentTree" => dictionary! { "Nums" => vec![Object::Integer(2), Object::Null] },
            }),
            true,
        );
        let page_id = doc.add_object(dictionary! { "Type" => "Page", "StructParents" => 3 });
        let pages_id = doc.add_object(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 });
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Parent", pages_id);
        let catalog_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_object_mut(catalog_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Pages", pages_id);
        let report = audit_accessibility_document(&doc);
        let parent = report
            .checks
            .iter()
            .find(|check| check.id == "structure.parent-tree")
            .unwrap();
        assert_eq!(parent.result, AccessibilityCheckResult::Fail);
        assert!(parent
            .evidence
            .iter()
            .any(|item| item.message.contains("has no ParentTree entry")));
    }

    #[test]
    fn mark_info_dictionary_entry_value_and_structure_are_distinct_facts() {
        let absent = audit_accessibility_document(&minimal_document());
        assert!(!absent.document_facts.has_mark_info);
        assert!(!absent.document_facts.has_marked_entry);
        assert!(!absent.document_facts.marked);

        let marked = audit_accessibility_document(&document_with_structure(
            Object::Dictionary(dictionary! { "Type" => "StructTreeRoot" }),
            true,
        ));
        assert!(marked.document_facts.has_mark_info);
        assert!(marked.document_facts.has_marked_entry);
        assert!(marked.document_facts.marked);
        assert!(marked.document_facts.has_structure_tree);

        let mut false_value = document_with_structure(
            Object::Dictionary(dictionary! { "Type" => "StructTreeRoot" }),
            false,
        );
        let false_report = audit_accessibility_document(&false_value);
        assert!(false_report.document_facts.has_mark_info);
        assert!(false_report.document_facts.has_marked_entry);
        assert!(!false_report.document_facts.marked);
        assert_eq!(
            result_for(&false_report, "tagging.mark-info").result,
            AccessibilityCheckResult::Warning
        );

        catalog_mut(&mut false_value).set("MarkInfo", Dictionary::new());
        let missing_entry = audit_accessibility_document(&false_value);
        assert!(missing_entry.document_facts.has_mark_info);
        assert!(!missing_entry.document_facts.has_marked_entry);
        assert!(!missing_entry.document_facts.marked);

        let mut marked_without_structure = minimal_document();
        catalog_mut(&mut marked_without_structure)
            .set("MarkInfo", dictionary! { "Marked" => true });
        let inconsistent = audit_accessibility_document(&marked_without_structure);
        assert!(!inconsistent.document_facts.has_structure_tree);
        assert!(inconsistent.document_facts.marked);
        assert_eq!(
            result_for(&inconsistent, "tagging.mark-info").result,
            AccessibilityCheckResult::Warning
        );
    }

    #[test]
    fn language_states_cover_missing_valid_empty_invalid_and_oversized_strings() {
        assert_eq!(text_value(None).0, AccessibilityTextState::Missing);
        assert_eq!(
            text_value(Some(&Object::String(Vec::new(), StringFormat::Literal))).0,
            AccessibilityTextState::Empty
        );
        assert_eq!(
            text_value(Some(&Object::String(
                b"  \t".to_vec(),
                StringFormat::Literal
            )))
            .0,
            AccessibilityTextState::WhitespaceOnly
        );
        assert!(valid_language_tag(Some("en-US")));
        assert!(!valid_language_tag(Some("en_US")));
        assert_eq!(
            text_value(Some(&Object::String(
                vec![b'x'; MAX_TEXT_BYTES + 1],
                StringFormat::Literal,
            )))
            .0,
            AccessibilityTextState::Invalid
        );
        let valid = audit_accessibility_document(&full_implemented_checks_document());
        assert_eq!(
            result_for(&valid, "document.language").result,
            AccessibilityCheckResult::Pass
        );
        let mut invalid = full_implemented_checks_document();
        catalog_mut(&mut invalid).set(
            "Lang",
            Object::String(b"en_US".to_vec(), StringFormat::Literal),
        );
        assert_eq!(
            result_for(&audit_accessibility_document(&invalid), "document.language").result,
            AccessibilityCheckResult::Fail
        );
    }

    #[test]
    fn metadata_title_and_display_doc_title_remain_independent() {
        let complete = audit_accessibility_document(&full_implemented_checks_document());
        assert_eq!(
            result_for(&complete, "metadata.document-title").result,
            AccessibilityCheckResult::Pass
        );
        assert_eq!(
            result_for(&complete, "metadata.display-document-title").result,
            AccessibilityCheckResult::Pass
        );

        let mut display_false = full_implemented_checks_document();
        catalog_mut(&mut display_false).set(
            "ViewerPreferences",
            dictionary! { "DisplayDocTitle" => false },
        );
        let report = audit_accessibility_document(&display_false);
        assert_eq!(
            result_for(&report, "metadata.document-title").result,
            AccessibilityCheckResult::Pass
        );
        assert_eq!(
            result_for(&report, "metadata.display-document-title").result,
            AccessibilityCheckResult::Fail
        );

        let mut no_title = full_implemented_checks_document();
        catalog_mut(&mut no_title).remove(b"Metadata");
        let report = audit_accessibility_document(&no_title);
        assert_eq!(
            result_for(&report, "metadata.document-title").result,
            AccessibilityCheckResult::Fail
        );
        assert_eq!(
            result_for(&report, "metadata.display-document-title").result,
            AccessibilityCheckResult::Pass
        );
    }

    #[test]
    fn pdfua_profiles_require_matching_versions_and_declarations_never_claim_conformance() {
        let ua1 = PdfUaDeclaration {
            part: Some("1".into()),
            revision: None,
            state: AccessibilityTextState::Present,
        };
        let ua2 = PdfUaDeclaration {
            part: Some("2".into()),
            revision: Some("2024".into()),
            state: AccessibilityTextState::Present,
        };
        assert_eq!(
            select_profile(Some("1.7"), &ua1),
            AccessibilityReferenceProfile::PdfUa1Reference
        );
        assert_eq!(
            select_profile(Some("2.0"), &ua2),
            AccessibilityReferenceProfile::PdfUa2Reference
        );
        assert_eq!(
            select_profile(Some("2.0"), &ua1),
            AccessibilityReferenceProfile::Undetermined
        );
        assert_eq!(
            select_profile(Some("1.7"), &ua2),
            AccessibilityReferenceProfile::Undetermined
        );
        assert_eq!(
            select_profile(
                Some("2.0"),
                &PdfUaDeclaration {
                    revision: Some("broken".into()),
                    ..ua2.clone()
                }
            ),
            AccessibilityReferenceProfile::Undetermined
        );

        let report = audit_accessibility_document(&full_implemented_checks_document());
        assert!(report.checks.iter().all(|check| !check.conformance_claim));
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::NoFindingsInImplementedChecks
        );

        let mut declaration_but_failed_check = full_implemented_checks_document();
        catalog_mut(&mut declaration_but_failed_check).set(
            "Lang",
            Object::String(b"invalid_tag".to_vec(), StringFormat::Literal),
        );
        let report = audit_accessibility_document(&declaration_but_failed_check);
        assert_eq!(
            result_for(&report, "document.pdfua-declaration").result,
            AccessibilityCheckResult::Pass
        );
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::FindingsDetected
        );

        let mut no_declaration = full_implemented_checks_document();
        metadata_stream_mut(&mut no_declaration).content = br#"<x:xmpmeta xmlns:x='adobe:ns:meta/' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:RDF><rdf:Description><dc:title><rdf:Alt><rdf:li xml:lang='x-default'>Accessible title</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#.to_vec();
        let report = audit_accessibility_document(&no_declaration);
        assert_eq!(
            result_for(&report, "document.pdfua-declaration").result,
            AccessibilityCheckResult::Fail
        );
        assert_eq!(
            report
                .checks
                .iter()
                .filter(|check| check.result == AccessibilityCheckResult::Fail)
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            vec!["document.pdfua-declaration"]
        );

        let mut ua2_document = full_implemented_checks_document();
        ua2_document.version = "2.0".to_string();
        metadata_stream_mut(&mut ua2_document).content = br#"<x:xmpmeta xmlns:x='adobe:ns:meta/' xmlns:pdfuaid='http://www.aiim.org/pdfua/ns/id/' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:RDF><rdf:Description><pdfuaid:part>2</pdfuaid:part><pdfuaid:rev>2024</pdfuaid:rev><dc:title><rdf:Alt><rdf:li xml:lang='x-default'>Accessible title</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#.to_vec();
        let report = audit_accessibility_document(&ua2_document);
        assert_eq!(
            report.reference_profile,
            AccessibilityReferenceProfile::PdfUa2Reference
        );
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );

        let mut malformed = full_implemented_checks_document();
        metadata_stream_mut(&mut malformed).content = b"<broken></mismatch>".to_vec();
        let report = audit_accessibility_document(&malformed);
        assert_eq!(
            result_for(&report, "document.pdfua-declaration").result,
            AccessibilityCheckResult::Indeterminate
        );
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );
    }

    #[test]
    fn figure_alt_states_are_distinct_and_quality_remains_human_review() {
        let cases = [
            (
                None,
                AccessibilityTextState::Missing,
                AccessibilityCheckResult::Fail,
            ),
            (
                Some(Vec::new()),
                AccessibilityTextState::Empty,
                AccessibilityCheckResult::Fail,
            ),
            (
                Some(b"  ".to_vec()),
                AccessibilityTextState::WhitespaceOnly,
                AccessibilityCheckResult::Fail,
            ),
            (
                Some(b"Chart description".to_vec()),
                AccessibilityTextState::Present,
                AccessibilityCheckResult::Pass,
            ),
            (
                Some(vec![b'x'; MAX_TEXT_BYTES + 1]),
                AccessibilityTextState::Invalid,
                AccessibilityCheckResult::Fail,
            ),
        ];
        for (alt, state, expected) in cases {
            let mut figure = dictionary! { "Type" => "StructElem", "S" => "Figure" };
            if let Some(bytes) = alt {
                figure.set("Alt", Object::String(bytes, StringFormat::Literal));
            }
            let root =
                dictionary! { "Type" => "StructTreeRoot", "K" => Object::Dictionary(figure) };
            let doc = document_with_structure(Object::Dictionary(root), true);
            let inspection = inspect_structure_tree_from_document(&doc).unwrap();
            assert_eq!(inspection.root.unwrap().children[0].alt_text_state, state);
            let report = audit_accessibility_document(&doc);
            assert_eq!(
                result_for(&report, "structure.figure-alternative-description").result,
                expected
            );
            assert!(report
                .human_review
                .iter()
                .any(|item| item.id == "human.figure-alt-quality"));
        }

        let figures = (0..256)
            .map(|index| {
                Object::Dictionary(dictionary! {
                    "Type" => "StructElem", "S" => "Figure",
                    "Alt" => Object::String(format!("Figure {index}").into_bytes(), StringFormat::Literal),
                })
            })
            .collect::<Vec<_>>();
        let root = dictionary! { "Type" => "StructTreeRoot", "K" => figures };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        assert_eq!(
            result_for(&report, "structure.figure-alternative-description").result,
            AccessibilityCheckResult::Pass
        );
        assert!(!serde_json::to_string(&report).unwrap().contains("score"));
    }

    #[test]
    fn dangling_children_are_visible_as_deterministic_unresolved_nodes() {
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "K" => Object::Reference((999, 0)),
        };
        let doc = document_with_structure(Object::Dictionary(root), true);
        let first = inspect_structure_tree_from_document(&doc).unwrap();
        let second = inspect_structure_tree_from_document(&doc).unwrap();
        let first_child = &first.root.as_ref().unwrap().children[0];
        assert_eq!(first_child.kind, AccessibilityStructureNodeKind::Unknown);
        assert_eq!(first_child.association_status, "unresolved");
        assert_eq!(first_child.object_id.as_deref(), Some("999 0 R"));
        assert_eq!(first_child.id, second.root.unwrap().children[0].id);
        assert!(first_child.findings[0].contains("Dangling structure reference"));
    }

    #[test]
    fn multi_node_cycles_and_non_active_duplicate_references_are_distinguished() {
        let mut cycle_doc = Document::with_version("1.7");
        let a = cycle_doc.new_object_id();
        let b = cycle_doc.new_object_id();
        cycle_doc.objects.insert(
            a,
            Object::Dictionary(dictionary! { "Type" => "StructElem", "S" => "P", "K" => b }),
        );
        cycle_doc.objects.insert(
            b,
            Object::Dictionary(dictionary! { "Type" => "StructElem", "S" => "P", "K" => a }),
        );
        let root = cycle_doc.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => a });
        let catalog =
            cycle_doc.add_object(dictionary! { "Type" => "Catalog", "StructTreeRoot" => root });
        cycle_doc.trailer.set("Root", catalog);
        assert!(inspect_structure_tree_from_document(&cycle_doc)
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.contains("cycle")));

        let mut duplicate_doc = Document::with_version("1.7");
        let shared = duplicate_doc.add_object(dictionary! { "Type" => "StructElem", "S" => "P" });
        let root = duplicate_doc.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => vec![Object::Reference(shared), Object::Reference(shared)] });
        let catalog =
            duplicate_doc.add_object(dictionary! { "Type" => "Catalog", "StructTreeRoot" => root });
        duplicate_doc.trailer.set("Root", catalog);
        let inspection = inspect_structure_tree_from_document(&duplicate_doc).unwrap();
        assert!(!inspection
            .findings
            .iter()
            .any(|finding| finding.contains("cycle")));
        let children = inspection.root.unwrap().children;
        assert_eq!(children.len(), 2);
        assert_ne!(children[0].id, children[1].id);
    }

    #[test]
    fn role_map_missing_type_unmapped_target_and_entry_limit_are_bounded_findings() {
        for child in [
            dictionary! { "Type" => "StructElem" },
            dictionary! { "Type" => "StructElem", "S" => "UnmappedCustom" },
        ] {
            let root = dictionary! { "Type" => "StructTreeRoot", "K" => Object::Dictionary(child) };
            let report = audit_accessibility_document(&document_with_structure(
                Object::Dictionary(root),
                true,
            ));
            assert_eq!(
                result_for(&report, "structure.role-mapping").result,
                AccessibilityCheckResult::Fail
            );
        }

        let mut role_map = Dictionary::new();
        for index in 0..=MAX_ROLE_MAP_ENTRIES {
            role_map.set(format!("Custom{index}"), Object::Name(b"P".to_vec()));
        }
        let root = dictionary! { "Type" => "StructTreeRoot", "RoleMap" => role_map };
        let inspection = inspect_structure_tree_from_document(&document_with_structure(
            Object::Dictionary(root),
            true,
        ))
        .unwrap();
        assert!(inspection.truncated);
        assert!(inspection
            .findings
            .iter()
            .any(|finding| finding.contains("RoleMap exceeds")));
    }

    #[test]
    fn parent_tree_duplicate_malformed_dangling_and_child_limit_are_findings() {
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "ParentTree" => dictionary! { "Nums" => vec![Object::Integer(1), Object::Null, Object::Integer(1), Object::Reference((999, 0))] },
        };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        let parent = result_for(&report, "structure.parent-tree");
        assert_eq!(parent.result, AccessibilityCheckResult::Fail);
        assert!(parent
            .evidence
            .iter()
            .any(|item| item.message.contains("duplicated")));
        assert!(parent
            .evidence
            .iter()
            .any(|item| item.message.contains("Dangling")));

        let oversized_kids = vec![Object::Null; MAX_CHILDREN_PER_NODE + 1];
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "ParentTree" => dictionary! { "Kids" => oversized_kids },
        };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        assert!(report.truncated);
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );
        assert!(result_for(&report, "structure.parent-tree")
            .evidence
            .iter()
            .any(|item| item.message.contains("child count")));
    }

    #[test]
    fn structure_child_and_parent_tree_entry_limits_produce_incomplete_reports() {
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "K" => vec![Object::Integer(0); MAX_CHILDREN_PER_NODE + 1],
        };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        assert!(report.truncated);
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );
        assert!(result_for(&report, "structure.integrity")
            .evidence
            .iter()
            .any(|item| item.message.contains("Child count")));

        let mut nums = Vec::with_capacity((MAX_PARENT_TREE_ENTRIES + 1) * 2);
        for index in 0..=MAX_PARENT_TREE_ENTRIES {
            nums.push(Object::Integer(i64::try_from(index).unwrap()));
            nums.push(Object::Null);
        }
        let root = dictionary! {
            "Type" => "StructTreeRoot",
            "ParentTree" => dictionary! { "Nums" => nums },
        };
        let report =
            audit_accessibility_document(&document_with_structure(Object::Dictionary(root), true));
        assert!(report.truncated);
        assert_eq!(
            result_for(&report, "structure.parent-tree").result,
            AccessibilityCheckResult::Indeterminate
        );
        assert!(result_for(&report, "structure.parent-tree")
            .evidence
            .iter()
            .any(|item| item.message.contains("entry limit")));
    }

    #[test]
    fn traversal_string_evidence_and_report_limits_fail_incomplete_and_bounded() {
        let mut depth = TraversalBudget::new();
        assert!(!depth.admit(MAX_STRUCTURE_DEPTH + 1, None));
        assert!(depth.truncated);

        let mut nodes = TraversalBudget::new();
        nodes.nodes = MAX_STRUCTURE_NODES;
        assert!(!nodes.admit(0, None));
        assert!(nodes
            .warnings
            .iter()
            .any(|item| item.contains("node limit")));

        let mut visited = TraversalBudget::new();
        visited.visited_objects = (0..MAX_VISITED_STRUCTURE_OBJECTS)
            .map(|index| (u32::try_from(index + 1).unwrap(), 0))
            .collect();
        assert!(!visited.admit(0, Some((u32::MAX, 0))));
        assert!(visited
            .warnings
            .iter()
            .any(|item| item.contains("visited-object")));

        assert_eq!(
            bounded_preview(&"x".repeat(MAX_PREVIEW_CHARS + 50))
                .chars()
                .count(),
            MAX_PREVIEW_CHARS
        );
        let bounded = check(
            "bounded",
            "Bounded",
            "test",
            AccessibilityCheckResult::Fail,
            AccessibilitySeverity::Info,
            AccessibilityAutomation::Machine,
            (0..MAX_EVIDENCE_PER_CHECK + 10)
                .map(|index| evidence(format!("evidence {index}")))
                .collect(),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(bounded.evidence.len(), MAX_EVIDENCE_PER_CHECK);

        let mut report = audit_accessibility_document(&full_implemented_checks_document());
        let original_size = serde_json::to_vec(&report).unwrap().len();
        enforce_report_size(&mut report, original_size - 1);
        assert!(report.truncated);
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::AuditIncomplete
        );
        assert!(report.checks.iter().all(|item| item.evidence.is_empty()));
        assert!(
            serde_json::to_vec(&audit_accessibility_document(
                &full_implemented_checks_document()
            ))
            .unwrap()
            .len()
                <= MAX_REPORT_BYTES
        );
    }

    #[test]
    fn not_checked_and_human_review_never_become_machine_passes() {
        let report = audit_accessibility_document(&full_implemented_checks_document());
        assert_eq!(
            report.summary_status,
            AccessibilitySummaryStatus::NoFindingsInImplementedChecks
        );
        assert!(!report.coverage.not_implemented_check_ids.is_empty());
        assert!(report
            .coverage
            .not_implemented_check_ids
            .iter()
            .all(|id| { result_for(&report, id).result == AccessibilityCheckResult::NotChecked }));
        assert!(report
            .human_review
            .iter()
            .all(|item| { item.result == AccessibilityCheckResult::HumanReviewRequired }));
        assert!(report
            .human_review
            .iter()
            .any(|item| item.id == "human.logical-reading-order"));
        assert!(report
            .human_review
            .iter()
            .any(|item| item.id == "human.color-contrast"));
        assert!(
            report
                .coverage
                .categories
                .iter()
                .any(|category| category.category == "annotations"
                    && category.status == "not_checked")
        );
        assert!(report
            .coverage
            .categories
            .iter()
            .any(|category| category.category == "forms" && category.status == "not_checked"));
        assert!(report
            .coverage
            .categories
            .iter()
            .any(|category| category.category == "text" && category.status == "not_checked"));
        assert!(report
            .coverage
            .categories
            .iter()
            .any(|category| category.category == "encryption" && category.status == "not_checked"));
    }
}
