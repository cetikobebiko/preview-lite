use crate::document_session::SessionJobOperationError;
use crate::sanitization::{
    control_descriptors, validate_inspection_report, SanitizationBlockingFinding,
    SanitizationControlInspection, SanitizationDocumentFacts, SanitizationFindingEvidence,
    SanitizationInspectionLimits, SanitizationInspectionReport, SanitizationInspectionWarning,
    MAX_SANITIZATION_ACTION_CHAIN_DEPTH, MAX_SANITIZATION_ARRAY_ELEMENTS,
    MAX_SANITIZATION_DECODED_STREAM_BYTES, MAX_SANITIZATION_DICTIONARY_ENTRIES,
    MAX_SANITIZATION_EMBEDDED_FILES, MAX_SANITIZATION_EVIDENCE_PER_CONTROL,
    MAX_SANITIZATION_FINDINGS, MAX_SANITIZATION_NAME_TREE_DEPTH, MAX_SANITIZATION_OBJECTS,
    MAX_SANITIZATION_RAW_SCAN_BYTES, MAX_SANITIZATION_TOTAL_DECODED_BYTES,
    MAX_SANITIZATION_TRAVERSAL_DEPTH, SANITIZATION_INSPECTION_PROFILE,
    SANITIZATION_INSPECTION_VERSION, SANITIZATION_POLICY_SCHEMA_VERSION,
    SANITIZATION_REPORT_SCHEMA_VERSION,
};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::{fs::File, io::Read};

type Checkpoint<'a> = &'a dyn Fn() -> Result<(), SessionJobOperationError>;

#[cfg(feature = "sanitization-test-support")]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationInspectionTestBehavior {
    #[default]
    Real,
    CrashDuringTraversal,
    HangDuringTraversal,
    HangDuringDecode,
}

#[derive(Clone, Copy)]
struct InspectionConfiguration {
    decoded_stream_limit: u64,
    total_decoded_limit: u64,
    #[cfg(feature = "sanitization-test-support")]
    test_behavior: SanitizationInspectionTestBehavior,
}

fn production_configuration() -> InspectionConfiguration {
    InspectionConfiguration {
        decoded_stream_limit: MAX_SANITIZATION_DECODED_STREAM_BYTES,
        total_decoded_limit: MAX_SANITIZATION_TOTAL_DECODED_BYTES,
        #[cfg(feature = "sanitization-test-support")]
        test_behavior: SanitizationInspectionTestBehavior::Real,
    }
}

#[derive(Default)]
struct FindingBucket {
    identities: BTreeSet<String>,
    evidence: Vec<SanitizationFindingEvidence>,
}

#[derive(Default)]
struct InspectionAccumulator {
    findings: BTreeMap<String, FindingBucket>,
    warnings: BTreeMap<String, SanitizationInspectionWarning>,
    total_findings: usize,
    embedded_file_identities: BTreeSet<String>,
}

impl InspectionAccumulator {
    fn add(
        &mut self,
        control_id: &str,
        identity: String,
        object_id: Option<ObjectId>,
        logical_path: &str,
        event_or_action_type: Option<&str>,
    ) -> Result<(), SessionJobOperationError> {
        let bucket = self.findings.entry(control_id.to_string()).or_default();
        if !bucket.identities.insert(identity) {
            return Ok(());
        }
        if self.total_findings >= MAX_SANITIZATION_FINDINGS {
            return Err(limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Sanitization finding sınırı aşıldı",
            ));
        }
        self.total_findings += 1;
        if bucket.evidence.len() < MAX_SANITIZATION_EVIDENCE_PER_CONTROL {
            bucket.evidence.push(SanitizationFindingEvidence {
                category: control_id.to_string(),
                object_id: object_id.map(format_object_id),
                logical_path: bounded_logical_path(logical_path),
                event_or_action_type: event_or_action_type.map(str::to_string),
            });
        }
        Ok(())
    }

    fn add_warning(&mut self, code: &str, message: &str) {
        if self.warnings.len() < crate::sanitization::MAX_SANITIZATION_WARNINGS {
            self.warnings.entry(code.to_string()).or_insert_with(|| {
                SanitizationInspectionWarning {
                    code: code.to_string(),
                    safe_message: message.to_string(),
                }
            });
        }
    }

    fn add_embedded_identity(
        &mut self,
        identity: String,
        object_id: Option<ObjectId>,
        path: &str,
    ) -> Result<(), SessionJobOperationError> {
        if self.embedded_file_identities.insert(identity.clone())
            && self.embedded_file_identities.len() > MAX_SANITIZATION_EMBEDDED_FILES
        {
            return Err(limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Sanitization embedded-file sınırı aşıldı",
            ));
        }
        self.add("embedded_files", identity, object_id, path, None)
    }
}

struct ObjectGraphScanner<'a> {
    document: &'a Document,
    input_digest: String,
    checkpoint: Checkpoint<'a>,
    visited: HashSet<ObjectId>,
    inspected_nodes: usize,
    action_count: usize,
    decoded_streams: HashSet<ObjectId>,
    decoded_bytes: u64,
    decoded_stream_limit: u64,
    total_decoded_limit: u64,
    #[cfg(feature = "sanitization-test-support")]
    test_behavior: SanitizationInspectionTestBehavior,
    #[cfg(feature = "sanitization-test-support")]
    test_marker_path: std::path::PathBuf,
    accumulator: InspectionAccumulator,
    facts: SanitizationDocumentFacts,
}

struct ScannerStreamFlags {
    encrypted: bool,
    object_streams_detected: bool,
    xref_streams_detected: bool,
}

impl<'a> ObjectGraphScanner<'a> {
    fn new(
        document: &'a Document,
        input_digest: String,
        checkpoint: Checkpoint<'a>,
        stream_flags: ScannerStreamFlags,
        configuration: InspectionConfiguration,
        #[cfg(feature = "sanitization-test-support")] test_marker_path: std::path::PathBuf,
    ) -> Result<Self, SessionJobOperationError> {
        let page_count = u32::try_from(document.get_pages().len()).map_err(|_| {
            limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Sanitization page count sınırı aşıldı",
            )
        })?;
        let ScannerStreamFlags {
            encrypted,
            object_streams_detected,
            xref_streams_detected,
        } = stream_flags;
        Ok(Self {
            document,
            input_digest,
            checkpoint,
            visited: HashSet::new(),
            inspected_nodes: 0,
            action_count: 0,
            decoded_streams: HashSet::new(),
            decoded_bytes: 0,
            decoded_stream_limit: configuration.decoded_stream_limit,
            total_decoded_limit: configuration.total_decoded_limit,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: configuration.test_behavior,
            #[cfg(feature = "sanitization-test-support")]
            test_marker_path,
            accumulator: InspectionAccumulator::default(),
            facts: SanitizationDocumentFacts {
                encrypted,
                object_streams_detected,
                xref_streams_detected,
                page_count,
                ..SanitizationDocumentFacts::default()
            },
        })
    }

    fn scan(mut self) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
        (self.checkpoint)()?;
        self.scan_trailer_facts()?;
        self.scan_catalog_roots()?;
        let trailer = Object::Dictionary(self.document.trailer.clone());
        self.walk_object(&trailer, "trailer", 0)?;

        let descriptors = control_descriptors()?;
        let mut controls = Vec::with_capacity(descriptors.len());
        let mut blocking_findings = Vec::new();
        for descriptor in descriptors.values() {
            let bucket = self.accumulator.findings.remove(&descriptor.control_id);
            let finding_count = bucket
                .as_ref()
                .map(|bucket| bucket.identities.len())
                .unwrap_or(0);
            let finding_count = u32::try_from(finding_count).map_err(|_| {
                limit_error(
                    "SANITIZATION_LIMIT_EXCEEDED",
                    "Sanitization finding count dönüştürülemedi",
                )
            })?;
            let present = finding_count > 0;
            let blocking_reason = if descriptor.blocking_for_minimum_profile && present {
                Some(blocking_message(&descriptor.control_id).to_string())
            } else {
                None
            };
            if let Some(reason) = &blocking_reason {
                blocking_findings.push(SanitizationBlockingFinding {
                    code: blocking_code(&descriptor.control_id).to_string(),
                    control_id: descriptor.control_id.clone(),
                    category: descriptor.category.clone(),
                    safe_message: reason.clone(),
                    evidence_count: finding_count,
                });
            }
            controls.push(SanitizationControlInspection {
                control_id: descriptor.control_id.clone(),
                category: descriptor.category.clone(),
                inspection_support: descriptor.inspection_support,
                future_removal_support: descriptor.future_removal_support,
                present,
                finding_count,
                expected_visual_impact: descriptor.expected_visual_impact.clone(),
                expected_functional_impact: descriptor.expected_functional_impact.clone(),
                blocking_reason,
                representative_evidence: bucket.map(|bucket| bucket.evidence).unwrap_or_default(),
            });
        }
        if !self.accumulator.findings.is_empty() {
            return Err(inspection_error(
                "Scanner canonical registry dışında finding üretti",
            ));
        }
        let report = SanitizationInspectionReport {
            schema_version: SANITIZATION_REPORT_SCHEMA_VERSION.to_string(),
            inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
            policy_schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            control_registry_version: crate::sanitization::SANITIZATION_CONTROL_REGISTRY_VERSION
                .to_string(),
            input_digest: self.input_digest,
            document_facts: self.facts,
            controls,
            blocking_findings,
            warnings: self.accumulator.warnings.into_values().collect(),
            limits: SanitizationInspectionLimits {
                visited_objects: u32::try_from(self.inspected_nodes).map_err(|_| {
                    limit_error(
                        "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                        "Sanitization visited-object sayısı dönüştürülemedi",
                    )
                })?,
                decoded_bytes: self.decoded_bytes,
                findings: u32::try_from(self.accumulator.total_findings).map_err(|_| {
                    limit_error(
                        "SANITIZATION_LIMIT_EXCEEDED",
                        "Sanitization total finding sayısı dönüştürülemedi",
                    )
                })?,
                truncated: false,
            },
        };
        validate_inspection_report(&report)?;
        Ok(report)
    }

    fn scan_trailer_facts(&mut self) -> Result<(), SessionJobOperationError> {
        if self.document.trailer.has(b"Info") {
            let object_id = reference_id(self.document.trailer.get(b"Info").ok());
            self.accumulator.add(
                "document_info",
                identity_for(object_id, "trailer/Info"),
                object_id,
                "trailer/Info",
                None,
            )?;
        }
        if self.document.trailer.has(b"ID") {
            self.facts.document_identifier_present = true;
            self.accumulator.add(
                "document_identifier",
                "trailer/ID".to_string(),
                None,
                "trailer/ID",
                None,
            )?;
        }
        if self.document.trailer.has(b"Prev") {
            self.facts.incremental_revisions_detected = true;
            self.accumulator.add(
                "incremental_revisions",
                "trailer/Prev".to_string(),
                None,
                "trailer/Prev",
                None,
            )?;
        }
        if self.facts.encrypted {
            self.accumulator.add(
                "encryption",
                "trailer/Encrypt".to_string(),
                reference_id(self.document.trailer.get(b"Encrypt").ok()),
                "trailer/Encrypt",
                None,
            )?;
        }
        Ok(())
    }

    fn scan_catalog_roots(&mut self) -> Result<(), SessionJobOperationError> {
        let catalog = self
            .document
            .catalog()
            .map_err(|_| inspection_error("PDF Catalog çözümlenemedi"))?;
        let open_action = catalog.get(b"OpenAction").ok().cloned();
        let additional_actions = catalog.get(b"AA").ok().cloned();
        let names = catalog.get(b"Names").ok().cloned();
        if let Some(open_action) = open_action {
            self.scan_action_root(
                &open_action,
                "Catalog/OpenAction",
                ActionOrigin::OpenAction,
                0,
                &mut HashSet::new(),
            )?;
        }
        if let Some(additional_actions) = additional_actions {
            self.scan_additional_actions(
                &additional_actions,
                "Catalog/AA",
                ActionOrigin::Additional,
            )?;
        }
        if let Some(names) = names {
            let names_dict = self.resolve_dictionary(&names, "Catalog/Names")?.clone();
            if let Ok(javascript) = names_dict.get(b"JavaScript") {
                self.scan_name_tree(
                    javascript,
                    "Catalog/Names/JavaScript",
                    NameTreeKind::JavaScript,
                    0,
                    &mut HashSet::new(),
                )?;
            }
            if let Ok(embedded) = names_dict.get(b"EmbeddedFiles") {
                self.scan_name_tree(
                    embedded,
                    "Catalog/Names/EmbeddedFiles",
                    NameTreeKind::EmbeddedFiles,
                    0,
                    &mut HashSet::new(),
                )?;
            }
        }
        Ok(())
    }

    fn walk_object(
        &mut self,
        object: &Object,
        path: &str,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        #[cfg(feature = "sanitization-test-support")]
        run_inspection_test_behavior(
            self.test_behavior,
            SanitizationInspectionTestPhase::Traversal,
            &self.test_marker_path,
        );
        (self.checkpoint)()?;
        if depth > MAX_SANITIZATION_TRAVERSAL_DEPTH {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization object graph depth sınırı aşıldı",
            ));
        }
        self.inspected_nodes = self.inspected_nodes.checked_add(1).ok_or_else(|| {
            limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization visited-object sayısı taştı",
            )
        })?;
        if self.inspected_nodes > MAX_SANITIZATION_OBJECTS {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization visited-object sınırı aşıldı",
            ));
        }
        match object {
            Object::Reference(id) => {
                if !self.visited.insert(*id) {
                    return Ok(());
                }
                let resolved = self.document.get_object(*id).map_err(|_| {
                    inspection_error("PDF object graph çözümlenemeyen reference içeriyor")
                })?;
                self.walk_object(resolved, &format!("object:{}:{}", id.0, id.1), depth + 1)
            }
            Object::Array(values) => {
                if values.len() > MAX_SANITIZATION_ARRAY_ELEMENTS {
                    return Err(limit_error(
                        "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                        "Sanitization array element sınırı aşıldı",
                    ));
                }
                for (index, value) in values.iter().enumerate() {
                    self.walk_object(value, &format!("{path}/[{index}]"), depth + 1)?;
                }
                Ok(())
            }
            Object::Dictionary(dictionary) => {
                self.inspect_dictionary(dictionary, path, None)?;
                self.walk_dictionary(dictionary, path, depth)
            }
            Object::Stream(stream) => {
                (self.checkpoint)()?;
                self.inspect_dictionary(&stream.dict, path, None)?;
                self.walk_dictionary(&stream.dict, path, depth)
            }
            _ => Ok(()),
        }
    }

    fn walk_dictionary(
        &mut self,
        dictionary: &Dictionary,
        path: &str,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        if dictionary.len() > MAX_SANITIZATION_DICTIONARY_ENTRIES {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization dictionary entry sınırı aşıldı",
            ));
        }
        for (key, value) in dictionary.iter() {
            self.walk_object(value, &safe_child_path(path, key), depth + 1)?;
        }
        Ok(())
    }

    fn inspect_dictionary(
        &mut self,
        dictionary: &Dictionary,
        path: &str,
        object_id: Option<ObjectId>,
    ) -> Result<(), SessionJobOperationError> {
        let object_id = object_id.or_else(|| object_id_from_logical_path(path));
        let type_name = dictionary
            .get(b"Type")
            .ok()
            .and_then(|value| value.as_name().ok());
        let subtype = dictionary
            .get(b"Subtype")
            .ok()
            .and_then(|value| value.as_name().ok());
        let identity = || identity_for(object_id, path);

        if type_name == Some(b"Catalog") && dictionary.has(b"Metadata") {
            self.accumulator.add(
                "xmp_metadata",
                identity_for(
                    reference_id(dictionary.get(b"Metadata").ok()),
                    "Catalog/Metadata",
                ),
                reference_id(dictionary.get(b"Metadata").ok()),
                "Catalog/Metadata",
                None,
            )?;
        }
        if type_name == Some(b"Page") && dictionary.has(b"Metadata") {
            self.accumulator.add(
                "page_metadata",
                identity_for(reference_id(dictionary.get(b"Metadata").ok()), path),
                reference_id(dictionary.get(b"Metadata").ok()),
                "Page/Metadata",
                None,
            )?;
        }
        if type_name == Some(b"Metadata") {
            self.accumulator
                .add("object_metadata", identity(), object_id, path, None)?;
        }
        if dictionary.has(b"PieceInfo") || dictionary.has(b"LastModified") {
            self.accumulator.add(
                "private_application_data",
                identity(),
                object_id,
                path,
                None,
            )?;
        }

        let is_filespec = type_name == Some(b"Filespec") || type_name == Some(b"FileSpec");
        if is_filespec || dictionary.has(b"EF") || dictionary.has(b"RF") {
            self.accumulator
                .add_embedded_identity(identity(), object_id, path)?;
            self.accumulator
                .add("embedded_file_metadata", identity(), object_id, path, None)?;
        }
        if type_name == Some(b"EmbeddedFile") {
            self.accumulator
                .add_embedded_identity(identity(), object_id, path)?;
        }
        if dictionary.has(b"AF") {
            self.scan_associated_files(dictionary.get(b"AF").ok(), path)?;
        }
        if subtype == Some(b"FileAttachment") {
            self.accumulator.add(
                "file_attachment_annotations",
                identity(),
                object_id,
                path,
                None,
            )?;
        }
        if dictionary.has(b"Collection") {
            self.accumulator
                .add("portfolio_collection", identity(), object_id, path, None)?;
        }

        if dictionary.has(b"AcroForm") {
            self.facts.acro_form_detected = true;
        }
        if dictionary.has(b"XFA") {
            self.facts.xfa_detected = true;
            self.accumulator
                .add("xfa", identity(), object_id, path, None)?;
        }
        if dictionary.has(b"CO") {
            self.facts.calculation_order_detected = true;
        }
        if let Ok(fields) = dictionary.get(b"Fields").and_then(Object::as_array) {
            self.facts.acro_form_detected = true;
            for (index, field) in fields.iter().enumerate() {
                let field_id = reference_id(Some(field));
                self.accumulator.add(
                    "all_form_fields",
                    identity_for(field_id, &format!("{path}/Fields/{index}")),
                    field_id,
                    &format!("{path}/Fields/[{index}]"),
                    None,
                )?;
            }
        }

        if dictionary.has(b"OCProperties")
            || type_name == Some(b"OCG")
            || type_name == Some(b"OCMD")
        {
            self.facts.optional_content_detected = true;
            self.accumulator
                .add("optional_content", identity(), object_id, path, None)?;
        }
        let is_annotation =
            type_name == Some(b"Annot") || dictionary.has(b"Rect") && subtype.is_some();
        if is_annotation {
            self.accumulator
                .add("all_annotations", identity(), object_id, path, None)?;
            if dictionary.has(b"OC") {
                self.facts.optional_content_detected = true;
                self.accumulator
                    .add("optional_content", identity(), object_id, path, None)?;
            }
        }
        if matches!(
            subtype,
            Some(b"RichMedia" | b"Screen" | b"Movie" | b"Sound" | b"3D")
        ) {
            self.facts.rich_media_detected = true;
            self.accumulator
                .add("rich_media", identity(), object_id, path, None)?;
        }

        let is_signature = type_name == Some(b"Sig")
            || dictionary
                .get(b"FT")
                .ok()
                .and_then(|value| value.as_name().ok())
                == Some(b"Sig")
            || dictionary.has(b"ByteRange");
        if is_signature {
            self.facts.signed = true;
            self.accumulator
                .add("digital_signatures", identity(), object_id, path, None)?;
        }
        if dictionary
            .get(b"TransformMethod")
            .ok()
            .and_then(|value| value.as_name().ok())
            == Some(b"DocMDP")
            || dictionary.has(b"DocMDP")
        {
            self.facts.certification_signature_detected = true;
        }
        if dictionary
            .get(b"TransformMethod")
            .ok()
            .and_then(|value| value.as_name().ok())
            == Some(b"FieldMDP")
        {
            self.facts.field_mdp_detected = true;
        }
        if type_name == Some(b"ObjStm") {
            self.facts.object_streams_detected = true;
        }
        if type_name == Some(b"XRef") {
            self.facts.xref_streams_detected = true;
        }

        let action_origin = if dictionary
            .get(b"FT")
            .ok()
            .and_then(|value| value.as_name().ok())
            .is_some()
            || subtype == Some(b"Widget")
        {
            ActionOrigin::Form
        } else if is_annotation {
            ActionOrigin::Annotation
        } else {
            ActionOrigin::Additional
        };
        if let Ok(action) = dictionary.get(b"A") {
            self.scan_action_root(
                &action.clone(),
                &format!("{path}/A"),
                action_origin,
                0,
                &mut HashSet::new(),
            )?;
        }
        if let Ok(additional) = dictionary.get(b"AA") {
            self.scan_additional_actions(
                &additional.clone(),
                &format!("{path}/AA"),
                action_origin,
            )?;
        }
        Ok(())
    }

    fn scan_associated_files(
        &mut self,
        object: Option<&Object>,
        path: &str,
    ) -> Result<(), SessionJobOperationError> {
        let Some(object) = object else {
            return Ok(());
        };
        let values: Vec<&Object> = match object {
            Object::Array(values) => values.iter().collect(),
            value => vec![value],
        };
        if values.len() > MAX_SANITIZATION_EMBEDDED_FILES {
            return Err(limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Associated-file sınırı aşıldı",
            ));
        }
        for (index, value) in values.into_iter().enumerate() {
            let object_id = reference_id(Some(value));
            self.accumulator.add(
                "associated_files",
                identity_for(object_id, &format!("{path}/AF/{index}")),
                object_id,
                &format!("{path}/AF/[{index}]"),
                None,
            )?;
            self.accumulator.add_embedded_identity(
                identity_for(object_id, &format!("{path}/AF/{index}")),
                object_id,
                &format!("{path}/AF/[{index}]"),
            )?;
        }
        Ok(())
    }

    fn scan_additional_actions(
        &mut self,
        object: &Object,
        path: &str,
        origin: ActionOrigin,
    ) -> Result<(), SessionJobOperationError> {
        (self.checkpoint)()?;
        let dictionary = self.resolve_dictionary(object, path)?.clone();
        if dictionary.len() > MAX_SANITIZATION_DICTIONARY_ENTRIES {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Additional-action event sınırı aşıldı",
            ));
        }
        for (event, action) in dictionary.iter() {
            let event_name = safe_event_name(event);
            self.scan_action_root(
                action,
                &format!("{path}/{event_name}"),
                origin,
                0,
                &mut HashSet::new(),
            )?;
        }
        Ok(())
    }

    fn scan_action_root(
        &mut self,
        object: &Object,
        path: &str,
        origin: ActionOrigin,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) -> Result<(), SessionJobOperationError> {
        (self.checkpoint)()?;
        if depth > MAX_SANITIZATION_ACTION_CHAIN_DEPTH {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization action-chain depth sınırı aşıldı",
            ));
        }
        if matches!(object, Object::Array(_)) && origin == ActionOrigin::OpenAction && depth == 0 {
            return Ok(());
        }
        let object_id = reference_id(Some(object));
        if let Some(id) = object_id {
            if !visited.insert(id) {
                self.accumulator.add_warning(
                    "SANITIZATION_ACTION_CYCLE_DETECTED",
                    "Döngüsel action chain bounded biçimde sonlandırıldı.",
                );
                return Ok(());
            }
        }
        let resolved = self
            .resolve_object(object)
            .ok_or_else(|| inspection_error("Action chain çözümlenemeyen reference içeriyor"))?
            .clone();
        if let Object::Array(actions) = &resolved {
            if actions.len() > MAX_SANITIZATION_ARRAY_ELEMENTS {
                return Err(limit_error(
                    "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                    "Action-chain array sınırı aşıldı",
                ));
            }
            for (index, action) in actions.iter().enumerate() {
                self.scan_action_root(
                    action,
                    &format!("{path}/Next/[{index}]"),
                    origin,
                    depth + 1,
                    visited,
                )?;
            }
            return Ok(());
        }
        let dictionary = resolved
            .as_dict()
            .map_err(|_| inspection_error("Action root dictionary değildir"))?;
        let action_type = dictionary
            .get(b"S")
            .ok()
            .and_then(|value| value.as_name().ok())
            .ok_or_else(|| inspection_error("Action dictionary /S type taşımıyor"))?;
        self.action_count = self
            .action_count
            .checked_add(1)
            .ok_or_else(|| limit_error("SANITIZATION_LIMIT_EXCEEDED", "Action count taştı"))?;
        if self.action_count > MAX_SANITIZATION_FINDINGS {
            return Err(limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Action count sınırı aşıldı",
            ));
        }
        let identity = identity_for(object_id, path);
        match action_type {
            b"JavaScript" => {
                if let Ok(payload) = dictionary.get(b"JS") {
                    self.inspect_decoded_action_payload(payload, &format!("{path}/JS"))?;
                }
                self.accumulator.add(
                    "document_javascript",
                    identity.clone(),
                    object_id,
                    path,
                    Some("JavaScript"),
                )?;
            }
            b"Launch" => self.accumulator.add(
                "launch_actions",
                identity.clone(),
                object_id,
                path,
                Some("Launch"),
            )?,
            b"URI" => self.accumulator.add(
                "uri_actions",
                identity.clone(),
                object_id,
                path,
                Some("URI"),
            )?,
            b"GoToR" | b"GoToE" => self.accumulator.add(
                "remote_navigation_actions",
                identity.clone(),
                object_id,
                path,
                Some(if action_type == b"GoToR" {
                    "GoToR"
                } else {
                    "GoToE"
                }),
            )?,
            b"SubmitForm" | b"ImportData" => self.accumulator.add(
                "form_submission_actions",
                identity.clone(),
                object_id,
                path,
                Some(if action_type == b"SubmitForm" {
                    "SubmitForm"
                } else {
                    "ImportData"
                }),
            )?,
            b"Rendition" => {
                self.facts.rich_media_detected = true;
                self.accumulator.add(
                    "rendition_actions",
                    identity.clone(),
                    object_id,
                    path,
                    Some("Rendition"),
                )?;
                self.accumulator.add(
                    "rich_media",
                    identity.clone(),
                    object_id,
                    path,
                    Some("Rendition"),
                )?;
            }
            b"GoTo" => {}
            _ => {
                return Err(inspection_error(
                    "Desteklenmeyen action type nedeniyle inspection tamamlanamadı",
                ))
            }
        }
        match origin {
            ActionOrigin::Additional => self.accumulator.add(
                "additional_actions",
                identity.clone(),
                object_id,
                path,
                Some("event_action"),
            )?,
            ActionOrigin::Annotation => {
                self.accumulator.add(
                    "annotation_actions",
                    identity.clone(),
                    object_id,
                    path,
                    Some("annotation_action"),
                )?;
                self.accumulator.add(
                    "additional_actions",
                    identity.clone(),
                    object_id,
                    path,
                    Some("annotation_action"),
                )?;
            }
            ActionOrigin::Form => {
                self.accumulator.add(
                    "form_actions",
                    identity.clone(),
                    object_id,
                    path,
                    Some("form_action"),
                )?;
                self.accumulator.add(
                    "additional_actions",
                    identity.clone(),
                    object_id,
                    path,
                    Some("form_action"),
                )?;
            }
            ActionOrigin::OpenAction | ActionOrigin::NameTree => {}
        }
        if let Ok(next) = dictionary.get(b"Next") {
            self.scan_action_root(next, &format!("{path}/Next"), origin, depth + 1, visited)?;
        }
        Ok(())
    }

    fn inspect_decoded_action_payload(
        &mut self,
        payload: &Object,
        _path: &str,
    ) -> Result<(), SessionJobOperationError> {
        (self.checkpoint)()?;
        let stream_id = reference_id(Some(payload));
        if stream_id.is_some_and(|id| self.decoded_streams.contains(&id)) {
            return Ok(());
        }
        let resolved = self
            .resolve_object(payload)
            .ok_or_else(|| inspection_error("JavaScript payload reference çözümlenemedi"))?;
        let Object::Stream(stream) = resolved else {
            return Ok(());
        };

        #[cfg(feature = "sanitization-test-support")]
        run_inspection_test_behavior(
            self.test_behavior,
            SanitizationInspectionTestPhase::Decode,
            &self.test_marker_path,
        );

        // lopdf 0.34 exposes whole-buffer decoding. The worker process and parent
        // timeout bound parser-level failures; these checks reject the result as
        // soon as the allocation-backed decode returns.
        let decoded = stream
            .get_plain_content()
            .map_err(|_| inspection_error("JavaScript stream payload'ı decode edilemedi"))?;
        (self.checkpoint)()?;
        let decoded_len = u64::try_from(decoded.len())
            .map_err(|_| decode_limit_error("Sanitization decoded stream byte sayısı taştı"))?;
        self.account_decoded_stream(decoded_len)?;
        if let Some(id) = stream_id {
            self.decoded_streams.insert(id);
        }
        Ok(())
    }

    fn account_decoded_stream(&mut self, decoded_len: u64) -> Result<(), SessionJobOperationError> {
        self.decoded_bytes = checked_decoded_total(
            self.decoded_bytes,
            decoded_len,
            self.decoded_stream_limit,
            self.total_decoded_limit,
        )?;
        Ok(())
    }

    fn scan_name_tree(
        &mut self,
        object: &Object,
        path: &str,
        kind: NameTreeKind,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) -> Result<(), SessionJobOperationError> {
        (self.checkpoint)()?;
        if depth > MAX_SANITIZATION_NAME_TREE_DEPTH {
            return Err(limit_error(
                "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                "Sanitization name-tree depth sınırı aşıldı",
            ));
        }
        if let Some(id) = reference_id(Some(object)) {
            if !visited.insert(id) {
                return Err(inspection_error(
                    "Döngüsel name tree nedeniyle inspection tamamlanamadı",
                ));
            }
        }
        let dictionary = self.resolve_dictionary(object, path)?.clone();
        if let Ok(limits) = dictionary.get(b"Limits") {
            if !matches!(limits, Object::Array(values) if values.len() == 2) {
                return Err(inspection_error("Name-tree /Limits yapısı malformed"));
            }
        }
        if let Ok(names) = dictionary.get(b"Names") {
            let names = names
                .as_array()
                .map_err(|_| inspection_error("Name-tree /Names array değildir"))?;
            if names.len() % 2 != 0 || names.len() > MAX_SANITIZATION_ARRAY_ELEMENTS {
                return Err(inspection_error(
                    "Name-tree /Names yapısı malformed veya limit dışı",
                ));
            }
            for (index, pair) in names.chunks_exact(2).enumerate() {
                let value = &pair[1];
                let value_path = format!("{path}/Names/[{}]", index * 2 + 1);
                match kind {
                    NameTreeKind::JavaScript => self.scan_action_root(
                        value,
                        &value_path,
                        ActionOrigin::NameTree,
                        0,
                        &mut HashSet::new(),
                    )?,
                    NameTreeKind::EmbeddedFiles => {
                        let object_id = reference_id(Some(value));
                        self.accumulator.add_embedded_identity(
                            identity_for(object_id, &value_path),
                            object_id,
                            &value_path,
                        )?;
                    }
                }
            }
        }
        if let Ok(kids) = dictionary.get(b"Kids") {
            let kids = kids
                .as_array()
                .map_err(|_| inspection_error("Name-tree /Kids array değildir"))?;
            if kids.len() > MAX_SANITIZATION_ARRAY_ELEMENTS {
                return Err(limit_error(
                    "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED",
                    "Name-tree /Kids sınırı aşıldı",
                ));
            }
            for (index, kid) in kids.iter().enumerate() {
                self.scan_name_tree(
                    kid,
                    &format!("{path}/Kids/[{index}]"),
                    kind,
                    depth + 1,
                    visited,
                )?;
            }
        }
        Ok(())
    }

    fn resolve_object<'b>(&'b self, object: &'b Object) -> Option<&'b Object> {
        match object {
            Object::Reference(id) => self.document.get_object(*id).ok(),
            value => Some(value),
        }
    }

    fn resolve_dictionary<'b>(
        &'b self,
        object: &'b Object,
        _path: &str,
    ) -> Result<&'b Dictionary, SessionJobOperationError> {
        match self.resolve_object(object) {
            Some(Object::Dictionary(dictionary)) => Ok(dictionary),
            Some(Object::Stream(stream)) => Ok(&stream.dict),
            _ => Err(inspection_error("Beklenen PDF dictionary çözümlenemedi")),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ActionOrigin {
    OpenAction,
    Additional,
    Annotation,
    Form,
    NameTree,
}

#[derive(Clone, Copy)]
enum NameTreeKind {
    JavaScript,
    EmbeddedFiles,
}

pub fn inspect_snapshot(
    snapshot_path: &Path,
    credential: Option<&str>,
    schema_version: &str,
    policy_schema_version: &str,
    inspection_profile: &str,
) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
    inspect_snapshot_with_configuration(
        snapshot_path,
        credential,
        schema_version,
        policy_schema_version,
        inspection_profile,
        &production_parent_enforced_cancellation,
        production_configuration(),
    )
}

fn production_parent_enforced_cancellation() -> Result<(), SessionJobOperationError> {
    // The production authority is the parent-owned worker process kill/wait path.
    // No cooperative cancellation IPC is claimed inside this ephemeral worker.
    Ok(())
}

#[cfg(test)]
pub(crate) fn inspect_snapshot_with_checkpoint(
    snapshot_path: &Path,
    credential: Option<&str>,
    schema_version: &str,
    policy_schema_version: &str,
    inspection_profile: &str,
    checkpoint: Checkpoint<'_>,
) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
    inspect_snapshot_with_configuration(
        snapshot_path,
        credential,
        schema_version,
        policy_schema_version,
        inspection_profile,
        checkpoint,
        production_configuration(),
    )
}

#[cfg(feature = "sanitization-test-support")]
pub fn inspect_snapshot_with_test_behavior(
    snapshot_path: &Path,
    credential: Option<&str>,
    schema_version: &str,
    policy_schema_version: &str,
    inspection_profile: &str,
    test_behavior: SanitizationInspectionTestBehavior,
) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
    let mut configuration = production_configuration();
    configuration.test_behavior = test_behavior;
    inspect_snapshot_with_configuration(
        snapshot_path,
        credential,
        schema_version,
        policy_schema_version,
        inspection_profile,
        &production_parent_enforced_cancellation,
        configuration,
    )
}

fn inspect_snapshot_with_configuration(
    snapshot_path: &Path,
    credential: Option<&str>,
    schema_version: &str,
    policy_schema_version: &str,
    inspection_profile: &str,
    checkpoint: Checkpoint<'_>,
    configuration: InspectionConfiguration,
) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
    if schema_version != SANITIZATION_REPORT_SCHEMA_VERSION
        || policy_schema_version != SANITIZATION_POLICY_SCHEMA_VERSION
        || inspection_profile != SANITIZATION_INSPECTION_PROFILE
    {
        return Err(inspection_error(
            "Sanitization worker request contract sürümü eşleşmedi",
        ));
    }
    checkpoint()?;
    let input_digest = crate::document_io::sha256_file(snapshot_path)
        .map_err(|_| inspection_error("Sanitization immutable snapshot digest hesaplanamadı"))?;
    let (object_streams_detected, xref_streams_detected) =
        scan_raw_structural_markers(snapshot_path, checkpoint)?;
    let mut document = Document::load(snapshot_path)
        .map_err(|_| inspection_error("Sanitization worker snapshot PDF'i açamadı"))?;
    let encrypted = document.is_encrypted();
    if encrypted {
        let password = credential.ok_or_else(|| {
            SessionJobOperationError::failed_with_code(
                "SANITIZATION_ENCRYPTED_INPUT_CREDENTIAL_REQUIRED",
                "Encrypted PDF inspection için session credential context gerekli",
            )
        })?;
        document.decrypt(password.as_bytes()).map_err(|_| {
            SessionJobOperationError::failed_with_code(
                "SANITIZATION_ENCRYPTED_INPUT_CREDENTIAL_REJECTED",
                "Session credential encrypted PDF snapshot'ını açamadı",
            )
        })?;
    }
    checkpoint()?;
    ObjectGraphScanner::new(
        &document,
        input_digest,
        checkpoint,
        ScannerStreamFlags {
            encrypted,
            object_streams_detected,
            xref_streams_detected,
        },
        configuration,
        #[cfg(feature = "sanitization-test-support")]
        snapshot_path.with_file_name("sanitization-test-phase"),
    )?
    .scan()
}

fn scan_raw_structural_markers(
    snapshot_path: &Path,
    checkpoint: Checkpoint<'_>,
) -> Result<(bool, bool), SessionJobOperationError> {
    const OBJECT_STREAM: &[u8] = b"/Type /ObjStm";
    const XREF_STREAM: &[u8] = b"/Type /XRef";
    const CARRY_BYTES: usize = 32;
    let mut file = File::open(snapshot_path)
        .map_err(|_| inspection_error("Sanitization worker raw structural scan başlatamadı"))?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut carry = Vec::new();
    let mut scanned = 0u64;
    let mut object_stream = false;
    let mut xref_stream = false;
    loop {
        checkpoint()?;
        let read = file.read(&mut buffer).map_err(|_| {
            inspection_error("Sanitization worker raw structural scan tamamlanamadı")
        })?;
        if read == 0 {
            break;
        }
        scanned = scanned.checked_add(read as u64).ok_or_else(|| {
            limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Sanitization raw scan byte sayısı taştı",
            )
        })?;
        if scanned > MAX_SANITIZATION_RAW_SCAN_BYTES {
            return Err(limit_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Sanitization raw structural scan byte sınırı aşıldı",
            ));
        }
        carry.extend_from_slice(&buffer[..read]);
        object_stream |= carry
            .windows(OBJECT_STREAM.len())
            .any(|window| window == OBJECT_STREAM);
        xref_stream |= carry
            .windows(XREF_STREAM.len())
            .any(|window| window == XREF_STREAM);
        if object_stream && xref_stream {
            break;
        }
        if carry.len() > CARRY_BYTES {
            carry.drain(..carry.len() - CARRY_BYTES);
        }
    }
    Ok((object_stream, xref_stream))
}

fn inspection_error(message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code("SANITIZATION_INSPECTION_FAILED", message)
}

fn limit_error(code: &'static str, message: &'static str) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

fn decode_limit_error(message: &'static str) -> SessionJobOperationError {
    limit_error("SANITIZATION_DECODE_LIMIT_EXCEEDED", message)
}

fn checked_decoded_total(
    current: u64,
    decoded_len: u64,
    per_stream_limit: u64,
    total_limit: u64,
) -> Result<u64, SessionJobOperationError> {
    if decoded_len > per_stream_limit {
        return Err(decode_limit_error(
            "Sanitization decoded stream byte sınırı aşıldı",
        ));
    }
    let total = current
        .checked_add(decoded_len)
        .ok_or_else(|| decode_limit_error("Sanitization total decoded byte sayısı taştı"))?;
    if total > total_limit {
        return Err(decode_limit_error(
            "Sanitization total decoded byte sınırı aşıldı",
        ));
    }
    Ok(total)
}

#[cfg(feature = "sanitization-test-support")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum SanitizationInspectionTestPhase {
    Traversal,
    Decode,
}

#[cfg(feature = "sanitization-test-support")]
fn run_inspection_test_behavior(
    behavior: SanitizationInspectionTestBehavior,
    phase: SanitizationInspectionTestPhase,
    marker_path: &Path,
) {
    let selected = match behavior {
        SanitizationInspectionTestBehavior::CrashDuringTraversal
        | SanitizationInspectionTestBehavior::HangDuringTraversal => {
            phase == SanitizationInspectionTestPhase::Traversal
        }
        SanitizationInspectionTestBehavior::HangDuringDecode => {
            phase == SanitizationInspectionTestPhase::Decode
        }
        SanitizationInspectionTestBehavior::Real => false,
    };
    if !selected {
        return;
    }
    let marker = match phase {
        SanitizationInspectionTestPhase::Traversal => b"traversal".as_slice(),
        SanitizationInspectionTestPhase::Decode => b"decode".as_slice(),
    };
    let _ = std::fs::write(marker_path, marker);
    match behavior {
        SanitizationInspectionTestBehavior::CrashDuringTraversal => std::process::abort(),
        SanitizationInspectionTestBehavior::HangDuringTraversal
        | SanitizationInspectionTestBehavior::HangDuringDecode => loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        },
        SanitizationInspectionTestBehavior::Real => {}
    }
}

fn format_object_id(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

fn reference_id(object: Option<&Object>) -> Option<ObjectId> {
    object.and_then(|value| value.as_reference().ok())
}

fn object_id_from_logical_path(path: &str) -> Option<ObjectId> {
    let raw = path.strip_prefix("object:")?;
    let mut parts = raw.split(':');
    let object_number = parts.next()?.parse().ok()?;
    let generation_number = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((object_number, generation_number))
}

fn identity_for(object_id: Option<ObjectId>, path: &str) -> String {
    object_id
        .map(format_object_id)
        .unwrap_or_else(|| bounded_logical_path(path))
}

fn bounded_logical_path(path: &str) -> String {
    const MAX_PATH_BYTES: usize = 384;
    if path.len() <= MAX_PATH_BYTES {
        return path.to_string();
    }
    let mut end = MAX_PATH_BYTES;
    while !path.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &path[..end])
}

fn safe_child_path(parent: &str, key: &[u8]) -> String {
    const KNOWN_KEYS: &[&[u8]] = &[
        b"Root",
        b"Info",
        b"Encrypt",
        b"Pages",
        b"Kids",
        b"Names",
        b"JavaScript",
        b"EmbeddedFiles",
        b"OpenAction",
        b"AA",
        b"A",
        b"Next",
        b"AcroForm",
        b"Fields",
        b"XFA",
        b"Metadata",
        b"AF",
        b"EF",
        b"RF",
        b"Annots",
        b"FS",
        b"Collection",
        b"OCProperties",
        b"OCGs",
        b"OC",
        b"Perms",
        b"Reference",
        b"V",
        b"Resources",
        b"XObject",
        b"StructTreeRoot",
        b"K",
        b"Parent",
        b"Contents",
        b"CO",
    ];
    let suffix = if KNOWN_KEYS.contains(&key) {
        String::from_utf8_lossy(key).into_owned()
    } else {
        "*".to_string()
    };
    bounded_logical_path(&format!("{parent}/{suffix}"))
}

fn safe_event_name(event: &[u8]) -> &'static str {
    match event {
        b"WC" => "WC",
        b"WS" => "WS",
        b"DS" => "DS",
        b"WP" => "WP",
        b"DP" => "DP",
        b"O" => "O",
        b"C" => "C",
        b"E" => "E",
        b"X" => "X",
        b"D" => "D",
        b"U" => "U",
        b"Fo" => "Fo",
        b"Bl" => "Bl",
        b"PO" => "PO",
        b"PC" => "PC",
        b"PV" => "PV",
        b"PI" => "PI",
        b"K" => "K",
        b"F" => "F",
        b"V" => "V",
        _ => "unknown_event",
    }
}

fn blocking_code(control_id: &str) -> &'static str {
    match control_id {
        "encryption" => "SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED",
        "digital_signatures" => "SANITIZATION_SIGNED_INPUT_REQUIRES_ACKNOWLEDGEMENT",
        "xfa" => "SANITIZATION_XFA_UNSUPPORTED",
        "optional_content" => "SANITIZATION_OPTIONAL_CONTENT_UNSUPPORTED",
        "rich_media" | "rendition_actions" => "SANITIZATION_MULTIMEDIA_UNSUPPORTED",
        _ => "SANITIZATION_BLOCKING_UNSUPPORTED_CONTENT",
    }
}

fn blocking_message(control_id: &str) -> &'static str {
    match control_id {
        "encryption" => "Encrypted input için gelecekteki sanitization transform desteği yok.",
        "digital_signatures" => {
            "İmzalı belge gelecekteki full rewrite işleminde imza geçerliliğini kaybedebilir."
        }
        "xfa" => {
            "Belge XFA içeriyor; sınırlı sanitization profili XFA'yı güvenli biçimde kaldıramaz."
        }
        "optional_content" => {
            "Belge optional content içeriyor; katman içeriği güvenli biçimde kaldırılamaz."
        }
        "rich_media" | "rendition_actions" => {
            "Belge desteklenmeyen multimedia içeriği veya eylemi içeriyor."
        }
        _ => "Belge minimum sanitization profili için desteklenmeyen içerik taşıyor.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, Stream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization")
            .join(name)
    }

    fn inspect(name: &str) -> SanitizationInspectionReport {
        inspect_snapshot(
            &fixture(name),
            if name == "encrypted.pdf" {
                Some("fixture-password")
            } else {
                None
            },
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap()
    }

    fn control<'a>(
        report: &'a SanitizationInspectionReport,
        id: &str,
    ) -> &'a SanitizationControlInspection {
        report
            .controls
            .iter()
            .find(|control| control.control_id == id)
            .unwrap()
    }

    fn inspect_javascript_streams(
        payloads: &[&[u8]],
        decoded_stream_limit: u64,
        total_decoded_limit: u64,
    ) -> Result<SanitizationInspectionReport, SessionJobOperationError> {
        let directory = tempfile::tempdir().unwrap();
        let mut document = Document::new();
        let pages = document.add_object(
            dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
        );
        let mut additional_actions = Dictionary::new();
        for (index, payload) in payloads.iter().enumerate() {
            let mut stream = Stream::new(Dictionary::new(), payload.to_vec());
            stream.compress().unwrap();
            let stream = document.add_object(stream);
            let action = document.add_object(dictionary! {
                "S" => "JavaScript",
                "JS" => Object::Reference(stream),
            });
            additional_actions.set(format!("T{index}"), Object::Reference(action));
        }
        let catalog = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(pages),
            "AA" => Object::Dictionary(additional_actions),
        });
        document.trailer.set("Root", Object::Reference(catalog));
        let path = directory.path().join("javascript-streams.pdf");
        document.save(&path).unwrap();
        inspect_snapshot_with_configuration(
            &path,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
            &|| Ok(()),
            InspectionConfiguration {
                decoded_stream_limit,
                total_decoded_limit,
                #[cfg(feature = "sanitization-test-support")]
                test_behavior: SanitizationInspectionTestBehavior::Real,
            },
        )
    }

    #[test]
    fn clean_fixture_is_strict_and_has_no_supported_findings() {
        let report = inspect("clean-minimal.pdf");
        assert!(report.controls.iter().all(|control| !control.present));
        assert!(report.blocking_findings.is_empty());
        validate_inspection_report(&report).unwrap();
    }

    #[test]
    fn javascript_paths_and_destination_negative_control_are_classified() {
        let report = inspect("javascript-actions.pdf");
        assert!(control(&report, "document_javascript").finding_count >= 5);
        assert!(control(&report, "additional_actions").present);
        assert!(control(&report, "annotation_actions").present);
        assert!(control(&report, "form_actions").present);
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("SANITIZATION_JS_PAYLOAD_001"));

        let destination = inspect("openaction-destination.pdf");
        assert!(!control(&destination, "document_javascript").present);
    }

    #[test]
    fn action_chain_is_typed_cycle_safe_and_local_goto_is_not_remote() {
        let report = inspect("action-chain-network.pdf");
        for id in [
            "launch_actions",
            "uri_actions",
            "remote_navigation_actions",
            "form_submission_actions",
        ] {
            assert!(control(&report, id).present, "missing {id}");
        }
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "SANITIZATION_ACTION_CYCLE_DETECTED"));

        let mut document = Document::new();
        let action = document
            .add_object(dictionary! { "S" => "GoTo", "D" => Object::Name(b"local".to_vec()) });
        let root = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference((3, 0)), "OpenAction" => Object::Reference(action) });
        document.objects.insert((3, 0), Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference((4, 0))], "Count" => 1 }));
        document.objects.insert((4, 0), Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => Object::Reference((3, 0)), "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()] }));
        document.trailer.set("Root", Object::Reference(root));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goto.pdf");
        document.save(&path).unwrap();
        let report = inspect_snapshot(
            &path,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        assert!(!control(&report, "remote_navigation_actions").present);
    }

    #[test]
    fn embedded_metadata_xfa_optional_multimedia_signature_encryption_and_history_are_reported() {
        let embedded = inspect("embedded-files-af.pdf");
        for id in [
            "embedded_files",
            "associated_files",
            "file_attachment_annotations",
            "portfolio_collection",
        ] {
            assert!(control(&embedded, id).present, "missing {id}");
        }
        let metadata = inspect("metadata-paths.pdf");
        for id in [
            "document_info",
            "xmp_metadata",
            "page_metadata",
            "object_metadata",
            "document_identifier",
            "private_application_data",
        ] {
            assert!(control(&metadata, id).present, "missing {id}");
        }
        let unsupported = inspect("unsupported-active-content.pdf");
        assert!(unsupported.document_facts.xfa_detected);
        assert!(unsupported.document_facts.optional_content_detected);
        assert!(unsupported.document_facts.rich_media_detected);
        assert!(unsupported.blocking_findings.len() >= 3);
        let signed = inspect("signature.pdf");
        assert!(signed.document_facts.signed);
        assert!(signed.document_facts.certification_signature_detected);
        assert!(signed.document_facts.field_mdp_detected);
        let encrypted = inspect("encrypted.pdf");
        assert!(encrypted.document_facts.encrypted);
        let incremental = inspect("incremental-revision.pdf");
        assert!(incremental.document_facts.incremental_revisions_detected);
        let streams = inspect("object-xref-stream.pdf");
        assert!(streams.document_facts.object_streams_detected);
        assert!(streams.document_facts.xref_streams_detected);
    }

    #[test]
    fn cancellation_checkpoint_aborts_without_partial_report() {
        let calls = AtomicUsize::new(0);
        let error = inspect_snapshot_with_checkpoint(
            &fixture("javascript-actions.pdf"),
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
            &|| {
                if calls.fetch_add(1, Ordering::SeqCst) > 5 {
                    Err(SessionJobOperationError::Cancelled(
                        crate::document_session::SessionJobCancellationReason::UserRequested,
                    ))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(matches!(error, SessionJobOperationError::Cancelled(_)));
        assert!(!matches!(error, SessionJobOperationError::Failed(_)));
    }

    #[test]
    fn decoded_stream_limits_account_exact_boundary_and_fail_closed() {
        let normal_payload = vec![b'A'; 32];
        let normal = inspect_javascript_streams(&[&normal_payload], 64, 256).unwrap();
        assert_eq!(normal.limits.decoded_bytes, 32);
        assert!(control(&normal, "document_javascript").present);
        assert!(!serde_json::to_string(&normal).unwrap().contains("AAAAAAAA"));

        let exact_payload = vec![b'B'; 64];
        let exact = inspect_javascript_streams(&[&exact_payload], 64, 256).unwrap();
        assert_eq!(exact.limits.decoded_bytes, 64);

        let oversized_payload = vec![b'C'; 65];
        let error = inspect_javascript_streams(&[&oversized_payload], 64, 256).unwrap_err();
        assert!(matches!(
            error,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));
    }

    #[test]
    fn total_decoded_stream_budget_and_overflow_fail_closed() {
        let first = vec![b'D'; 40];
        let second = vec![b'E'; 40];
        let error = inspect_javascript_streams(&[&first, &second], 64, 64).unwrap_err();
        assert!(matches!(
            error,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));

        let overflow = checked_decoded_total(u64::MAX - 1, 2, 2, u64::MAX).unwrap_err();
        assert!(matches!(
            overflow,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));
    }

    #[test]
    fn cyclic_name_tree_and_deep_graph_fail_closed_with_typed_errors() {
        let directory = tempfile::tempdir().unwrap();
        let mut document = Document::new();
        let node = document.new_object_id();
        document.objects.insert(
            node,
            Object::Dictionary(dictionary! { "Kids" => vec![Object::Reference(node)] }),
        );
        let pages = document.add_object(
            dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
        );
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages, "Names" => dictionary! { "JavaScript" => Object::Reference(node) } });
        document.trailer.set("Root", catalog);
        let cyclic_path = directory.path().join("cyclic-name-tree.pdf");
        document.save(&cyclic_path).unwrap();
        let cycle_error = inspect_snapshot(
            &cyclic_path,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap_err();
        assert!(matches!(cycle_error, SessionJobOperationError::Failed(_)));

        let mut deep = Document::new();
        let mut child = Object::Null;
        for _ in 0..=MAX_SANITIZATION_TRAVERSAL_DEPTH {
            let id = deep.add_object(dictionary! { "Child" => child });
            child = Object::Reference(id);
        }
        let pages = deep.add_object(
            dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
        );
        let catalog = deep
            .add_object(dictionary! { "Type" => "Catalog", "Pages" => pages, "Custom" => child });
        deep.trailer.set("Root", catalog);
        let deep_path = directory.path().join("deep.pdf");
        deep.save(&deep_path).unwrap();
        let depth_error = inspect_snapshot(
            &deep_path,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap_err();
        match depth_error {
            SessionJobOperationError::Failed(failure) => {
                assert_eq!(failure.code, "SANITIZATION_TRAVERSAL_LIMIT_EXCEEDED")
            }
            _ => panic!("expected typed traversal failure"),
        }
    }

    #[test]
    fn report_validator_rejects_cross_field_contradictions_and_credentials_never_serialize() {
        let mut report = inspect("clean-minimal.pdf");
        report.controls[0].present = true;
        assert!(validate_inspection_report(&report).is_err());

        let encrypted = inspect("encrypted.pdf");
        let encoded = serde_json::to_string(&encrypted).unwrap();
        assert!(!encoded.contains("fixture-password"));
        assert!(!encoded.contains("SANITIZATION_ENCRYPTED_PAYLOAD_001"));
    }
}
