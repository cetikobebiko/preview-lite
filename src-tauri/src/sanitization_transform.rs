use crate::document_session::{SessionJobOperationError, SessionJobOutcome};
use crate::sanitization::{
    inspection_digest, validate_sanitization_policy, validate_transform_report,
    validate_transform_report_for_policy, validate_verification_report_for_transform,
    SanitizationControlTransformResult, SanitizationDocumentInvariants,
    SanitizationFullRewriteEvidence, SanitizationInspectionReport, SanitizationInspectionWarning,
    SanitizationPageInvariant, SanitizationPolicy, SanitizationTransformEvidence,
    SanitizationTransformReport, SanitizationTransformStatus, SanitizationVerificationReport,
    MAX_SANITIZATION_ACTION_CHAIN_DEPTH, MAX_SANITIZATION_DECODED_STREAM_BYTES,
    MAX_SANITIZATION_EVIDENCE_PER_CONTROL, MAX_SANITIZATION_OBJECTS,
    MAX_SANITIZATION_TOTAL_DECODED_BYTES, MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES,
    SANITIZATION_CONTROL_REGISTRY_VERSION, SANITIZATION_INSPECTION_PROFILE,
    SANITIZATION_INSPECTION_VERSION, SANITIZATION_POLICY_SCHEMA_VERSION,
    SANITIZATION_REPORT_SCHEMA_VERSION, SANITIZATION_TRANSFORM_OUTPUT_NAME,
    SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION,
};
use lopdf::{Dictionary, Document, Object, ObjectId};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::NamedTempFile;

#[cfg(feature = "sanitization-test-support")]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationTransformTestBehavior {
    #[default]
    Real,
    CrashDuringMutation,
    HangDuringMutation,
    HangDuringSerialization,
}

fn transform_error(code: &'static str, message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

fn map_job_error(error: SessionJobOperationError) -> crate::job_worker::JobFailure {
    match error {
        SessionJobOperationError::Failed(failure) => {
            crate::job_worker::JobFailure::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(reason) => {
            crate::job_worker::JobFailure::new("SESSION_JOB_CANCELLED", format!("{reason:?}"))
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ActionOrigin {
    OpenAction,
    Additional,
    Annotation,
    Form,
    Next,
}

#[derive(Default)]
struct MutationEvidence {
    counts: BTreeMap<String, u32>,
    evidence: BTreeMap<String, Vec<SanitizationTransformEvidence>>,
    decoded_bytes: u64,
}

impl MutationEvidence {
    fn record(
        &mut self,
        controls: impl IntoIterator<Item = &'static str>,
        path: impl Into<String> + Clone,
        object_id: Option<ObjectId>,
        payload_sha256: Option<String>,
    ) -> Result<(), SessionJobOperationError> {
        for control in controls {
            let count = self.counts.entry(control.to_string()).or_default();
            *count = count.checked_add(1).ok_or_else(|| {
                transform_error("SANITIZATION_LIMIT_EXCEEDED", "Mutation count overflow")
            })?;
            let bucket = self.evidence.entry(control.to_string()).or_default();
            if bucket.len() < MAX_SANITIZATION_EVIDENCE_PER_CONTROL {
                bucket.push(SanitizationTransformEvidence {
                    logical_path: path.clone().into(),
                    object_id: object_id
                        .map(|(number, generation)| format!("{number} {generation} R")),
                    payload_sha256: payload_sha256.clone(),
                });
            }
        }
        Ok(())
    }

    fn fingerprint_payload(
        &mut self,
        document: &Document,
        object: &Object,
    ) -> Result<Option<String>, SessionJobOperationError> {
        let resolved = resolve_object(document, object).ok_or_else(|| {
            transform_error(
                "SANITIZATION_TRANSFORM_FAILED",
                "Payload reference çözümlenemedi",
            )
        })?;
        let (bytes, decoded_stream) = match resolved {
            Object::String(bytes, _) => (bytes.clone(), false),
            Object::Stream(stream) => (
                stream.get_plain_content().map_err(|_| {
                    transform_error(
                        "SANITIZATION_TRANSFORM_FAILED",
                        "Payload stream decode edilemedi",
                    )
                })?,
                true,
            ),
            _ => return Ok(None),
        };
        self.account_fingerprint_bytes(
            u64::try_from(bytes.len()).map_err(|_| {
                transform_error(
                    "SANITIZATION_DECODE_LIMIT_EXCEEDED",
                    "Decoded payload byte count overflow",
                )
            })?,
            decoded_stream,
        )?;
        Ok(Some(crate::hex_lower(Sha256::digest(bytes))))
    }

    // lopdf exposes decoded streams as whole buffers. This is therefore
    // post-decode enforcement: reject before accounting/fingerprinting, but do
    // not claim to prevent lopdf's initial allocation.
    fn account_fingerprint_bytes(
        &mut self,
        payload_bytes: u64,
        decoded_stream: bool,
    ) -> Result<(), SessionJobOperationError> {
        if decoded_stream && payload_bytes > MAX_SANITIZATION_DECODED_STREAM_BYTES {
            return Err(transform_error(
                "SANITIZATION_DECODE_LIMIT_EXCEEDED",
                "Transform decoded stream byte sınırı aşıldı",
            ));
        }
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(payload_bytes)
            .ok_or_else(|| {
                transform_error(
                    "SANITIZATION_DECODE_LIMIT_EXCEEDED",
                    "Transform total decoded byte sayısı taştı",
                )
            })?;
        if self.decoded_bytes > MAX_SANITIZATION_TOTAL_DECODED_BYTES {
            return Err(transform_error(
                "SANITIZATION_DECODE_LIMIT_EXCEEDED",
                "Transform payload fingerprint total budget aşıldı",
            ));
        }
        Ok(())
    }
}

struct Mutator<'a> {
    document: &'a mut Document,
    /// Input's last trailer has /Prev; read from the raw bytes because lopdf 0.45 strips it.
    previous_revision: bool,
    requested: BTreeSet<String>,
    evidence: MutationEvidence,
    visited_action_objects: HashSet<ObjectId>,
}

impl<'a> Mutator<'a> {
    fn requested(&self, control: &str) -> bool {
        self.requested.contains(control)
    }

    fn mutate(mut self) -> Result<(MutationEvidence, u32), SessionJobOperationError> {
        self.mutate_trailer()?;
        self.mutate_catalog_names()?;

        let object_ids: Vec<ObjectId> = self.document.objects.keys().copied().collect();
        if object_ids.len() > MAX_SANITIZATION_OBJECTS {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Transform object count sınırı aşıldı",
            ));
        }
        for object_id in object_ids {
            let Some(mut object) = self.document.objects.get(&object_id).cloned() else {
                continue;
            };
            self.mutate_object(&mut object, Some(object_id), 0)?;
            self.document.objects.insert(object_id, object);
        }

        self.document.trailer.remove(b"Prev");
        self.document.trailer.remove(b"XRefStm");
        let pruned = self.document.prune_objects();
        let pruned_count = u32::try_from(pruned.len()).map_err(|_| {
            transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Pruned object count overflow",
            )
        })?;
        validate_no_dangling_references(self.document)?;
        Ok((self.evidence, pruned_count))
    }

    fn mutate_trailer(&mut self) -> Result<(), SessionJobOperationError> {
        if self.requested("incremental_revisions") && self.previous_revision {
            self.evidence
                .record(["incremental_revisions"], "Trailer/Prev", None, None)?;
            self.document.trailer.remove(b"Prev");
            self.document.trailer.remove(b"XRefStm");
        }
        if self.requested("document_info") {
            if let Ok(info) = self.document.trailer.get(b"Info").cloned() {
                let digest = self.evidence.fingerprint_payload(self.document, &info)?;
                self.evidence.record(
                    ["document_info"],
                    "Trailer/Info",
                    reference_id(&info),
                    digest,
                )?;
                self.document.trailer.remove(b"Info");
            }
        }
        Ok(())
    }

    fn mutate_catalog_names(&mut self) -> Result<(), SessionJobOperationError> {
        let names = self
            .document
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get(b"Names").ok())
            .cloned();
        let Some(names) = names else { return Ok(()) };
        match names {
            Object::Reference(id) => {
                let mut dictionary = self
                    .document
                    .get_dictionary(id)
                    .map_err(|_| {
                        transform_error(
                            "SANITIZATION_TRANSFORM_FAILED",
                            "Catalog Names çözümlenemedi",
                        )
                    })?
                    .clone();
                self.mutate_names_dictionary(&mut dictionary, Some(id))?;
                if dictionary.is_empty() {
                    self.document
                        .catalog_mut()
                        .map_err(|_| {
                            transform_error(
                                "SANITIZATION_TRANSFORM_FAILED",
                                "Catalog mutable değil",
                            )
                        })?
                        .remove(b"Names");
                } else {
                    self.document
                        .objects
                        .insert(id, Object::Dictionary(dictionary));
                }
            }
            Object::Dictionary(mut dictionary) => {
                self.mutate_names_dictionary(&mut dictionary, None)?;
                if dictionary.is_empty() {
                    self.document
                        .catalog_mut()
                        .map_err(|_| {
                            transform_error(
                                "SANITIZATION_TRANSFORM_FAILED",
                                "Catalog mutable değil",
                            )
                        })?
                        .remove(b"Names");
                } else {
                    self.document
                        .catalog_mut()
                        .map_err(|_| {
                            transform_error(
                                "SANITIZATION_TRANSFORM_FAILED",
                                "Catalog mutable değil",
                            )
                        })?
                        .set("Names", Object::Dictionary(dictionary));
                }
            }
            _ => {
                return Err(transform_error(
                    "SANITIZATION_TRANSFORM_FAILED",
                    "Catalog Names dictionary değildir",
                ))
            }
        }
        Ok(())
    }

    fn mutate_names_dictionary(
        &mut self,
        dictionary: &mut Dictionary,
        object_id: Option<ObjectId>,
    ) -> Result<(), SessionJobOperationError> {
        if self.requested("document_javascript") {
            if let Ok(branch) = dictionary.get(b"JavaScript").cloned() {
                let digest = first_payload_fingerprint(self.document, &branch, &mut self.evidence)?;
                dictionary.remove(b"JavaScript");
                self.evidence.record(
                    ["document_javascript"],
                    "Catalog/Names/JavaScript",
                    object_id,
                    digest,
                )?;
            }
        }
        if self.requested("embedded_files") {
            if let Ok(branch) = dictionary.get(b"EmbeddedFiles").cloned() {
                let digest = first_payload_fingerprint(self.document, &branch, &mut self.evidence)?;
                dictionary.remove(b"EmbeddedFiles");
                self.evidence.record(
                    ["embedded_files"],
                    "Catalog/Names/EmbeddedFiles",
                    object_id,
                    digest,
                )?;
            }
        }
        Ok(())
    }

    fn mutate_object(
        &mut self,
        object: &mut Object,
        object_id: Option<ObjectId>,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        if depth > crate::sanitization::MAX_SANITIZATION_TRAVERSAL_DEPTH {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Transform traversal depth sınırı aşıldı",
            ));
        }
        match object {
            Object::Dictionary(dictionary) => self.mutate_dictionary(dictionary, object_id, depth),
            Object::Stream(stream) => self.mutate_dictionary(&mut stream.dict, object_id, depth),
            Object::Array(values) => {
                for value in values {
                    if !matches!(value, Object::Reference(_)) {
                        self.mutate_object(value, None, depth + 1)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn mutate_dictionary(
        &mut self,
        dictionary: &mut Dictionary,
        object_id: Option<ObjectId>,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        let type_name = dictionary_name(dictionary, b"Type");
        let subtype = dictionary_name(dictionary, b"Subtype");
        let is_page = type_name.as_deref() == Some(b"Page");
        let is_catalog = type_name.as_deref() == Some(b"Catalog");
        let is_annotation = type_name.as_deref() == Some(b"Annot")
            || (dictionary.has(b"Rect") && subtype.is_some());
        let is_form = dictionary.has(b"FT") || subtype.as_deref() == Some(b"Widget");

        if dictionary.has(b"Metadata") {
            let control = if is_catalog {
                Some("xmp_metadata")
            } else if is_page {
                Some("page_metadata")
            } else {
                Some("object_metadata")
            };
            if control.is_some_and(|control| self.requested(control)) {
                let metadata = dictionary.get(b"Metadata").unwrap().clone();
                let digest =
                    first_payload_fingerprint(self.document, &metadata, &mut self.evidence)?;
                dictionary.remove(b"Metadata");
                let mut credited = vec![control.unwrap()];
                if self.requested("object_metadata") {
                    credited.push("object_metadata");
                }
                credited.sort_unstable();
                credited.dedup();
                self.evidence.record(
                    credited,
                    if is_catalog {
                        "Catalog/Metadata"
                    } else if is_page {
                        "Page/Metadata"
                    } else {
                        "Object/Metadata"
                    },
                    object_id,
                    digest,
                )?;
            }
        }

        if (self.requested("embedded_files") || self.requested("associated_files"))
            && dictionary.has(b"AF")
        {
            let af = dictionary.get(b"AF").unwrap().clone();
            let digest = first_payload_fingerprint(self.document, &af, &mut self.evidence)?;
            dictionary.remove(b"AF");
            let mut controls = vec!["associated_files"];
            if self.requested("embedded_files") {
                controls.push("embedded_files");
            }
            controls.retain(|control| self.requested(control));
            self.evidence
                .record(controls, "Object/AF", object_id, digest)?;
        }

        let is_filespec = matches!(type_name.as_deref(), Some(b"Filespec") | Some(b"FileSpec"));
        if self.requested("embedded_files")
            && (is_filespec || dictionary.has(b"EF") || dictionary.has(b"RF"))
        {
            for key in [b"EF".as_slice(), b"RF".as_slice()] {
                if let Ok(payload) = dictionary.get(key).cloned() {
                    let digest =
                        first_payload_fingerprint(self.document, &payload, &mut self.evidence)?;
                    dictionary.remove(key);
                    self.evidence.record(
                        ["embedded_files"],
                        format!("FileSpec/{}", String::from_utf8_lossy(key)),
                        object_id,
                        digest,
                    )?;
                }
            }
        }

        if is_page
            && (self.requested("file_attachment_annotations") || self.requested("embedded_files"))
        {
            if let Ok(annots) = dictionary
                .get(b"Annots")
                .and_then(Object::as_array)
                .cloned()
            {
                let mut retained = Vec::with_capacity(annots.len());
                for annotation in annots {
                    if object_dictionary_name(self.document, &annotation, b"Subtype").as_deref()
                        == Some(b"FileAttachment")
                    {
                        let mut controls = Vec::new();
                        if self.requested("file_attachment_annotations") {
                            controls.push("file_attachment_annotations");
                        }
                        if self.requested("embedded_files") {
                            controls.push("embedded_files");
                        }
                        self.evidence.record(
                            controls,
                            "Page/Annots/FileAttachment",
                            reference_id(&annotation),
                            None,
                        )?;
                    } else {
                        retained.push(annotation);
                    }
                }
                if retained.is_empty() {
                    dictionary.remove(b"Annots");
                } else {
                    dictionary.set("Annots", retained);
                }
            }
        }

        if is_catalog {
            self.mutate_action_key(
                dictionary,
                b"OpenAction",
                ActionOrigin::OpenAction,
                object_id,
                depth,
            )?;
        }
        let origin = if is_form {
            ActionOrigin::Form
        } else if is_annotation {
            ActionOrigin::Annotation
        } else {
            ActionOrigin::Additional
        };
        self.mutate_action_key(dictionary, b"A", origin, object_id, depth)?;
        self.mutate_additional_actions(dictionary, origin, object_id, depth)?;
        if is_form && self.requested("form_actions") && dictionary.has(b"CO") {
            dictionary.remove(b"CO");
            self.evidence
                .record(["form_actions"], "AcroForm/CO", object_id, None)?;
        }
        if dictionary.has(b"S") {
            self.mutate_action_key(dictionary, b"Next", ActionOrigin::Next, object_id, depth)?;
        }

        let keys: Vec<Vec<u8>> = dictionary.iter().map(|(key, _)| key.clone()).collect();
        for key in keys {
            if matches!(
                key.as_slice(),
                b"A" | b"AA" | b"OpenAction" | b"Next" | b"Metadata" | b"AF" | b"EF" | b"RF"
            ) {
                continue;
            }
            if let Ok(value) = dictionary.get_mut(&key) {
                if !matches!(value, Object::Reference(_)) {
                    self.mutate_object(value, None, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    fn mutate_additional_actions(
        &mut self,
        dictionary: &mut Dictionary,
        origin: ActionOrigin,
        owner_id: Option<ObjectId>,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        let Ok(aa) = dictionary.get(b"AA").cloned() else {
            return Ok(());
        };
        let mut aa_dictionary = resolve_dictionary(self.document, &aa)
            .ok_or_else(|| {
                transform_error(
                    "SANITIZATION_TRANSFORM_FAILED",
                    "Additional actions dictionary çözümlenemedi",
                )
            })?
            .clone();
        let keys: Vec<Vec<u8>> = aa_dictionary.iter().map(|(key, _)| key.clone()).collect();
        for key in keys {
            let mut action = aa_dictionary.get(&key).unwrap().clone();
            if self.should_remove_by_origin(origin) {
                self.record_action_removal(
                    &action,
                    origin,
                    owner_id,
                    format!("Object/AA/{}", String::from_utf8_lossy(&key)),
                )?;
                aa_dictionary.remove(&key);
            } else if !self.mutate_action_value(&mut action, origin, depth + 1)? {
                aa_dictionary.remove(&key);
            } else {
                aa_dictionary.set(key, action);
            }
        }
        if aa_dictionary.is_empty() {
            dictionary.remove(b"AA");
        } else if let Object::Reference(id) = aa {
            self.document
                .objects
                .insert(id, Object::Dictionary(aa_dictionary));
        } else {
            dictionary.set("AA", Object::Dictionary(aa_dictionary));
        }
        Ok(())
    }

    fn mutate_action_key(
        &mut self,
        dictionary: &mut Dictionary,
        key: &[u8],
        origin: ActionOrigin,
        owner_id: Option<ObjectId>,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        let Ok(mut action) = dictionary.get(key).cloned() else {
            return Ok(());
        };
        if key == b"OpenAction" && matches!(action, Object::Array(_)) {
            return Ok(());
        }
        if self.should_remove_by_origin(origin) {
            self.record_action_removal(
                &action,
                origin,
                owner_id,
                format!("Object/{}", String::from_utf8_lossy(key)),
            )?;
            dictionary.remove(key);
        } else if !self.mutate_action_value(&mut action, origin, depth + 1)? {
            dictionary.remove(key);
        } else {
            dictionary.set(key, action);
        }
        Ok(())
    }

    fn should_remove_by_origin(&self, origin: ActionOrigin) -> bool {
        match origin {
            ActionOrigin::Additional => self.requested("additional_actions"),
            ActionOrigin::Annotation => {
                self.requested("annotation_actions") || self.requested("additional_actions")
            }
            ActionOrigin::Form => {
                self.requested("form_actions") || self.requested("additional_actions")
            }
            _ => false,
        }
    }

    fn mutate_action_value(
        &mut self,
        action: &mut Object,
        origin: ActionOrigin,
        depth: usize,
    ) -> Result<bool, SessionJobOperationError> {
        if depth > MAX_SANITIZATION_ACTION_CHAIN_DEPTH {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Action traversal depth sınırı aşıldı",
            ));
        }
        if let Object::Array(values) = action {
            let mut retained = Vec::with_capacity(values.len());
            for mut value in std::mem::take(values) {
                if self.mutate_action_value(&mut value, origin, depth + 1)? {
                    retained.push(value);
                }
            }
            *values = retained;
            return Ok(!values.is_empty());
        }
        let action_type = object_dictionary_name(self.document, action, b"S").ok_or_else(|| {
            transform_error(
                "SANITIZATION_TRANSFORM_FAILED",
                "Action /S type çözümlenemedi",
            )
        })?;
        if self.action_type_requested(&action_type) {
            self.record_action_removal(action, origin, reference_id(action), "Action")?;
            return Ok(false);
        }
        if let Object::Reference(id) = action {
            if !self.visited_action_objects.insert(*id) {
                return Ok(true);
            }
        }
        match action {
            Object::Reference(id) => {
                let mut dictionary = self
                    .document
                    .get_dictionary(*id)
                    .map_err(|_| {
                        transform_error(
                            "SANITIZATION_TRANSFORM_FAILED",
                            "Action reference dictionary değildir",
                        )
                    })?
                    .clone();
                self.mutate_action_key(
                    &mut dictionary,
                    b"Next",
                    ActionOrigin::Next,
                    Some(*id),
                    depth,
                )?;
                self.document
                    .objects
                    .insert(*id, Object::Dictionary(dictionary));
            }
            Object::Dictionary(dictionary) => {
                self.mutate_action_key(dictionary, b"Next", ActionOrigin::Next, None, depth)?;
            }
            _ => {
                return Err(transform_error(
                    "SANITIZATION_TRANSFORM_FAILED",
                    "Action dictionary değildir",
                ))
            }
        }
        Ok(true)
    }

    fn action_type_requested(&self, action_type: &[u8]) -> bool {
        match action_type {
            b"JavaScript" => self.requested("document_javascript"),
            b"Launch" => self.requested("launch_actions"),
            b"URI" => self.requested("uri_actions"),
            b"GoToR" | b"GoToE" => self.requested("remote_navigation_actions"),
            b"SubmitForm" | b"ImportData" => self.requested("form_submission_actions"),
            b"GoTo" => false,
            _ => false,
        }
    }

    fn record_action_removal(
        &mut self,
        action: &Object,
        origin: ActionOrigin,
        object_id: Option<ObjectId>,
        path: impl Into<String> + Clone,
    ) -> Result<(), SessionJobOperationError> {
        let path = path.into();
        self.record_action_tree(action, origin, object_id, &path, 0, &mut HashSet::new())
    }

    fn record_action_tree(
        &mut self,
        action: &Object,
        origin: ActionOrigin,
        object_id: Option<ObjectId>,
        path: &str,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) -> Result<(), SessionJobOperationError> {
        if depth > MAX_SANITIZATION_ACTION_CHAIN_DEPTH {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Removed action inventory depth sınırı aşıldı",
            ));
        }
        if let Object::Array(values) = action {
            for (index, value) in values.iter().enumerate() {
                self.record_action_tree(
                    value,
                    origin,
                    reference_id(value),
                    &format!("{path}/[{index}]"),
                    depth + 1,
                    visited,
                )?;
            }
            return Ok(());
        }
        let resolved_id = reference_id(action).or(object_id);
        if resolved_id.is_some_and(|id| !visited.insert(id)) {
            return Ok(());
        }
        let action_type = object_dictionary_name(self.document, action, b"S").ok_or_else(|| {
            transform_error(
                "SANITIZATION_TRANSFORM_FAILED",
                "Removed action /S type taşımıyor",
            )
        })?;
        let mut controls = Vec::new();
        match action_type.as_slice() {
            b"JavaScript" if self.requested("document_javascript") => {
                controls.push("document_javascript")
            }
            b"Launch" if self.requested("launch_actions") => controls.push("launch_actions"),
            b"URI" if self.requested("uri_actions") => controls.push("uri_actions"),
            b"GoToR" | b"GoToE" if self.requested("remote_navigation_actions") => {
                controls.push("remote_navigation_actions")
            }
            b"SubmitForm" | b"ImportData" if self.requested("form_submission_actions") => {
                controls.push("form_submission_actions")
            }
            _ => {}
        }
        match origin {
            ActionOrigin::Additional if self.requested("additional_actions") => {
                controls.push("additional_actions")
            }
            ActionOrigin::Annotation => {
                if self.requested("annotation_actions") {
                    controls.push("annotation_actions");
                }
                if self.requested("additional_actions") {
                    controls.push("additional_actions");
                }
            }
            ActionOrigin::Form => {
                if self.requested("form_actions") {
                    controls.push("form_actions");
                }
                if self.requested("additional_actions") {
                    controls.push("additional_actions");
                }
            }
            _ => {}
        }
        let payload_digest = if action_type == b"JavaScript" {
            resolve_dictionary(self.document, action)
                .and_then(|dictionary| dictionary.get(b"JS").ok())
                .cloned()
                .map(|payload| self.evidence.fingerprint_payload(self.document, &payload))
                .transpose()?
                .flatten()
        } else {
            None
        };
        if !controls.is_empty() {
            controls.sort_unstable();
            controls.dedup();
            self.evidence
                .record(controls, path.to_string(), resolved_id, payload_digest)?;
        }
        let next = resolve_dictionary(self.document, action)
            .and_then(|dictionary| dictionary.get(b"Next").ok())
            .cloned();
        if let Some(next) = next {
            self.record_action_tree(
                &next,
                origin,
                reference_id(&next),
                &format!("{path}/Next"),
                depth + 1,
                visited,
            )?;
        }
        Ok(())
    }
}

fn dictionary_name(dictionary: &Dictionary, key: &[u8]) -> Option<Vec<u8>> {
    dictionary
        .get(key)
        .ok()?
        .as_name()
        .ok()
        .map(ToOwned::to_owned)
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        value => Some(value),
    }
}

fn resolve_dictionary<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve_object(document, object)?.as_dict().ok()
}

fn object_dictionary_name(document: &Document, object: &Object, key: &[u8]) -> Option<Vec<u8>> {
    dictionary_name(resolve_dictionary(document, object)?, key)
}

fn reference_id(object: &Object) -> Option<ObjectId> {
    if let Object::Reference(id) = object {
        Some(*id)
    } else {
        None
    }
}

fn first_payload_fingerprint(
    document: &Document,
    root: &Object,
    evidence: &mut MutationEvidence,
) -> Result<Option<String>, SessionJobOperationError> {
    let mut stack = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut nodes = 0usize;
    let mut string_fallback = None;
    while let Some(object) = stack.pop() {
        nodes += 1;
        if nodes > MAX_SANITIZATION_OBJECTS {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Fingerprint graph sınırı aşıldı",
            ));
        }
        if let Object::Reference(id) = object {
            if visited.insert(id) {
                let resolved = document.get_object(id).map_err(|_| {
                    transform_error(
                        "SANITIZATION_TRANSFORM_FAILED",
                        "Fingerprint graph dangling reference içeriyor",
                    )
                })?;
                match resolved {
                    Object::Stream(_) => {
                        return evidence.fingerprint_payload(document, resolved);
                    }
                    Object::String(_, _) if string_fallback.is_none() => {
                        string_fallback = evidence.fingerprint_payload(document, resolved)?;
                    }
                    _ => {}
                }
                stack.push(resolved.clone());
            }
            continue;
        }
        match &object {
            Object::Stream(_) => return evidence.fingerprint_payload(document, &object),
            Object::String(_, _) if string_fallback.is_none() => {
                string_fallback = evidence.fingerprint_payload(document, &object)?;
            }
            _ => {}
        }
        match object {
            Object::Dictionary(dictionary) => {
                stack.extend(dictionary.iter().map(|(_, value)| value.clone()))
            }
            Object::Array(values) => stack.extend(values),
            _ => {}
        }
    }
    Ok(string_fallback)
}

fn inherited_page_value<'a>(
    document: &'a Document,
    page_id: ObjectId,
    key: &[u8],
) -> Result<&'a Object, SessionJobOperationError> {
    let mut current = page_id;
    let mut visited = HashSet::new();
    for _ in 0..=crate::sanitization::MAX_SANITIZATION_TRAVERSAL_DEPTH {
        if !visited.insert(current) {
            return Err(transform_error(
                "SANITIZATION_TRANSFORM_FAILED",
                "Input page tree cycle içeriyor",
            ));
        }
        let dictionary = document.get_dictionary(current).map_err(|_| {
            transform_error(
                "SANITIZATION_TRANSFORM_FAILED",
                "Input page tree dangling reference içeriyor",
            )
        })?;
        if let Ok(value) = dictionary.get(key) {
            return Ok(value);
        }
        current = dictionary
            .get(b"Parent")
            .and_then(Object::as_reference)
            .map_err(|_| {
                transform_error(
                    "SANITIZATION_TRANSFORM_FAILED",
                    "Input page geometry inheritance tamamlanamadı",
                )
            })?;
    }
    Err(transform_error(
        "SANITIZATION_LIMIT_EXCEEDED",
        "Input page tree traversal limiti aşıldı",
    ))
}

fn page_box_size(
    document: &Document,
    page_id: ObjectId,
) -> Result<(f32, f32), SessionJobOperationError> {
    let value = inherited_page_value(document, page_id, b"CropBox")
        .or_else(|_| inherited_page_value(document, page_id, b"MediaBox"))?;
    let value = resolve_object(document, value).ok_or_else(|| {
        transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input page box çözümlenemedi",
        )
    })?;
    let array = value.as_array().map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input page box array değildir",
        )
    })?;
    if array.len() != 4 {
        return Err(transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input page box dört koordinat taşımıyor",
        ));
    }
    let values: Result<Vec<f32>, _> = array.iter().map(Object::as_float).collect();
    let values = values.map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input page box numeric değildir",
        )
    })?;
    let width = (values[2] - values[0]).abs();
    let height = (values[3] - values[1]).abs();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input page box geçersizdir",
        ));
    }
    Ok((width, height))
}

fn count_local_goto_actions(document: &Document) -> u32 {
    document
        .objects
        .values()
        .filter_map(|object| match object {
            Object::Dictionary(dictionary) => Some(dictionary),
            Object::Stream(stream) => Some(&stream.dict),
            _ => None,
        })
        .filter(|dictionary| dictionary_name(dictionary, b"S").as_deref() == Some(b"GoTo"))
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn count_named_destinations(document: &Document) -> u32 {
    document
        .objects
        .values()
        .filter_map(|object| match object {
            Object::Dictionary(dictionary) => dictionary.get(b"Names").ok(),
            Object::Stream(stream) => stream.dict.get(b"Names").ok(),
            _ => None,
        })
        .filter_map(|value| resolve_object(document, value))
        .filter_map(|value| value.as_array().ok())
        .map(|array| array.len() / 2)
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn capture_expected_document_invariants(
    document: &Document,
) -> Result<SanitizationDocumentInvariants, SessionJobOperationError> {
    let catalog = document.catalog().map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_FAILED",
            "Input catalog çözümlenemedi",
        )
    })?;
    let mut pages = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        let (width_points, height_points) = page_box_size(document, page_id)?;
        let rotation_degrees = document
            .get_dictionary(page_id)
            .ok()
            .and_then(|page| page.get(b"Rotate").ok())
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(0)
            .rem_euclid(360) as i32;
        // lopdf 0.34 also fell back to raw bytes on decode failure; only the Result is gone.
        let content = document.get_page_content(page_id);
        pages.push(SanitizationPageInvariant {
            page_index: page_number - 1,
            width_points,
            height_points,
            rotation_degrees,
            content_sha256: crate::hex_lower(Sha256::digest(content)),
        });
    }
    let form_field_count = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|form| form.get(b"Fields").ok())
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_array().ok())
        .map(Vec::len)
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX);
    Ok(SanitizationDocumentInvariants {
        page_count: pages.len().try_into().map_err(|_| {
            transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Input page invariant count aşıldı",
            )
        })?,
        pages,
        named_destination_count: count_named_destinations(document),
        local_goto_action_count: count_local_goto_actions(document),
        form_field_count,
        outline_root_present: catalog.has(b"Outlines"),
    })
}

fn validate_no_dangling_references(document: &Document) -> Result<(), SessionJobOperationError> {
    fn walk(
        document: &Document,
        object: &Object,
        nodes: &mut usize,
    ) -> Result<(), SessionJobOperationError> {
        *nodes += 1;
        if *nodes > MAX_SANITIZATION_OBJECTS * 20 {
            return Err(transform_error(
                "SANITIZATION_LIMIT_EXCEEDED",
                "Reference validation sınırı aşıldı",
            ));
        }
        match object {
            Object::Reference(id) => {
                if !document.objects.contains_key(id) {
                    return Err(transform_error(
                        "SANITIZATION_TRANSFORM_FAILED",
                        "Transform dangling reference üretti",
                    ));
                }
            }
            Object::Array(values) => {
                for value in values {
                    walk(document, value, nodes)?;
                }
            }
            Object::Dictionary(dictionary) => {
                for (_, value) in dictionary.iter() {
                    walk(document, value, nodes)?;
                }
            }
            Object::Stream(stream) => {
                for (_, value) in stream.dict.iter() {
                    walk(document, value, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut nodes = 0;
    walk(
        document,
        &Object::Dictionary(document.trailer.clone()),
        &mut nodes,
    )?;
    for object in document.objects.values() {
        walk(document, object, &mut nodes)?;
    }
    Ok(())
}

pub(crate) struct SanitizationTransformExecutionArgs<'a> {
    pub job_dir: &'a Path,
    pub snapshot_path: &'a Path,
    pub credential: Option<&'a str>,
    pub content_generation: u64,
    pub policy: SanitizationPolicy,
    pub expected_input_digest: &'a str,
    pub expected_inspection_digest: &'a str,
    pub output_name: &'a str,
    #[cfg(feature = "sanitization-test-support")]
    pub test_behavior: SanitizationTransformTestBehavior,
}

pub(crate) fn execute_transform(
    args: SanitizationTransformExecutionArgs<'_>,
) -> Result<SanitizationTransformReport, crate::job_worker::JobFailure> {
    let SanitizationTransformExecutionArgs {
        job_dir,
        snapshot_path,
        credential,
        content_generation,
        policy,
        expected_input_digest,
        expected_inspection_digest,
        output_name,
        #[cfg(feature = "sanitization-test-support")]
        test_behavior,
    } = args;
    validate_sanitization_policy(&policy).map_err(map_job_error)?;
    if output_name != SANITIZATION_TRANSFORM_OUTPUT_NAME {
        return Err(crate::job_worker::JobFailure::new(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Sanitization transform output adı allowlisted internal ad değildir",
        ));
    }
    let actual_input_digest = crate::document_io::sha256_file(snapshot_path).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_STALE_INSPECTION",
            format!("Snapshot digest okunamadı: {error}"),
        )
    })?;
    if actual_input_digest != expected_input_digest {
        return Err(crate::job_worker::JobFailure::new(
            "SANITIZATION_STALE_INSPECTION",
            "Immutable snapshot digest inspection authority ile eşleşmedi",
        ));
    }
    let inspection = crate::sanitization_inspection::inspect_snapshot(
        snapshot_path,
        credential,
        SANITIZATION_REPORT_SCHEMA_VERSION,
        SANITIZATION_POLICY_SCHEMA_VERSION,
        SANITIZATION_INSPECTION_PROFILE,
    )
    .map_err(map_job_error)?;
    let actual_inspection_digest = inspection_digest(&inspection).map_err(map_job_error)?;
    if actual_inspection_digest != expected_inspection_digest
        || inspection.input_digest != actual_input_digest
    {
        return Err(crate::job_worker::JobFailure::new(
            "SANITIZATION_STALE_INSPECTION",
            "Inspection digest current snapshot authority ile eşleşmedi",
        ));
    }
    if inspection.document_facts.encrypted {
        return Err(crate::job_worker::JobFailure::new(
            "SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED",
            "Encrypted input transform desteklenmiyor",
        ));
    }
    if let Some(blocker) =
        crate::sanitization::unresolved_blocking_findings(&inspection, &policy, true)
            .map_err(map_job_error)?
            .into_iter()
            .next()
    {
        return Err(crate::job_worker::JobFailure::new(
            if blocker.control_id == "digital_signatures" {
                "SANITIZATION_SIGNATURE_ACKNOWLEDGEMENT_REQUIRED"
            } else {
                "SANITIZATION_UNSUPPORTED_CONTENT"
            },
            blocker.safe_message.clone(),
        ));
    }

    #[cfg(feature = "sanitization-test-support")]
    run_transform_test_behavior(
        job_dir,
        test_behavior,
        SanitizationTransformTestPhase::Mutation,
    );

    let input = std::fs::read(snapshot_path).map_err(|_| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            "Worker snapshot PDF'i okunamadı",
        )
    })?;
    let mut document = Document::load_mem(&input).map_err(|_| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            "Worker snapshot PDF object graph'ini açamadı",
        )
    })?;
    // An unreadable trailer counts as a previous revision so its removal is recorded.
    let previous_revision =
        crate::lopdf_compat::trailer_history(&input, &document).is_none_or(|history| history.prev);
    drop(input);
    let expected_document_invariants =
        capture_expected_document_invariants(&document).map_err(map_job_error)?;
    let requested = policy.controls.iter().cloned().collect();
    let (mutation, pruned_count) = Mutator {
        document: &mut document,
        previous_revision,
        requested,
        evidence: MutationEvidence::default(),
        visited_action_objects: HashSet::new(),
    }
    .mutate()
    .map_err(map_job_error)?;

    let output_path = checked_private_output_path(job_dir, output_name)?;
    #[cfg(feature = "sanitization-test-support")]
    run_transform_test_behavior(
        job_dir,
        test_behavior,
        SanitizationTransformTestPhase::Serialization,
    );
    let staging = NamedTempFile::new_in(job_dir).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            format!("Private staging oluşturulamadı: {error}"),
        )
    })?;
    document.save(staging.path()).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            format!("Full rewrite serialize edilemedi: {error}"),
        )
    })?;
    staging.as_file().sync_all().map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            format!("Private staging flush edilemedi: {error}"),
        )
    })?;
    let output_size_bytes = staging
        .as_file()
        .metadata()
        .map_err(|error| {
            crate::job_worker::JobFailure::new(
                "SANITIZATION_TRANSFORM_FAILED",
                format!("Artifact metadata okunamadı: {error}"),
            )
        })?
        .len();
    if output_size_bytes == 0 || output_size_bytes > MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES {
        return Err(crate::job_worker::JobFailure::new(
            "SANITIZATION_LIMIT_EXCEEDED",
            "Transform artifact size sınırı geçersiz",
        ));
    }
    let output_digest = crate::document_io::sha256_file(staging.path()).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            format!("Output digest hesaplanamadı: {error}"),
        )
    })?;
    let output = std::fs::read(staging.path()).map_err(|_| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            "Private artifact okunamadı",
        )
    })?;
    let reopened = Document::load_mem(&output).map_err(|_| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            "Private artifact parser ile yeniden açılamadı",
        )
    })?;
    // lopdf 0.45 strips /Prev and /XRefStm on load; the raw trailer is authoritative, and
    // a trailer that cannot be located is not a proven full rewrite.
    if crate::lopdf_compat::trailer_history(&output, &reopened).is_none_or(|history| history.any())
    {
        return Err(crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            "Full rewrite history pointer taşıyor",
        ));
    }
    staging.persist_noclobber(&output_path).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SANITIZATION_TRANSFORM_FAILED",
            format!("Private artifact promote edilemedi: {}", error.error),
        )
    })?;

    let controls = build_control_results(&inspection, &policy, mutation)?;
    let signed = inspection.document_facts.signed;
    let report = SanitizationTransformReport {
        schema_version: SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION.to_string(),
        policy_schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
        control_registry_version: SANITIZATION_CONTROL_REGISTRY_VERSION.to_string(),
        profile: policy.profile,
        input_digest: actual_input_digest,
        output_digest,
        output_size_bytes,
        output_relative_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
        inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
        inspection_digest: actual_inspection_digest,
        content_generation,
        controls,
        expected_document_invariants,
        full_rewrite_evidence: SanitizationFullRewriteEvidence {
            source_bytes_copied: false,
            incremental_save_used: false,
            active_prev_present: false,
            active_xref_stm_present: false,
            parser_reopen_passed: true,
            unreachable_objects_pruned: pruned_count,
            self_check_passed: true,
        },
        warnings: if signed {
            vec![SanitizationInspectionWarning {
                code: "SANITIZATION_SIGNATURES_INVALIDATED".to_string(),
                safe_message: "Full rewrite signatures invalidated and certification is lost."
                    .to_string(),
            }]
        } else {
            Vec::new()
        },
        signatures_invalidated: signed,
        certification_lost: signed && inspection.document_facts.certification_signature_detected,
        status: "transformed_unverified".to_string(),
    };
    validate_transform_report(&report).map_err(map_job_error)?;
    Ok(report)
}

#[cfg(feature = "sanitization-test-support")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum SanitizationTransformTestPhase {
    Mutation,
    Serialization,
}

#[cfg(feature = "sanitization-test-support")]
fn run_transform_test_behavior(
    job_dir: &Path,
    behavior: SanitizationTransformTestBehavior,
    phase: SanitizationTransformTestPhase,
) {
    let active = matches!(
        (behavior, phase),
        (
            SanitizationTransformTestBehavior::CrashDuringMutation,
            SanitizationTransformTestPhase::Mutation
        ) | (
            SanitizationTransformTestBehavior::HangDuringMutation,
            SanitizationTransformTestPhase::Mutation
        ) | (
            SanitizationTransformTestBehavior::HangDuringSerialization,
            SanitizationTransformTestPhase::Serialization
        )
    );
    if !active {
        return;
    }
    let marker = match phase {
        SanitizationTransformTestPhase::Mutation => b"mutation".as_slice(),
        SanitizationTransformTestPhase::Serialization => b"serialization".as_slice(),
    };
    let _ = fs::write(job_dir.join("sanitization-transform-test-phase"), marker);
    match behavior {
        SanitizationTransformTestBehavior::CrashDuringMutation => std::process::abort(),
        SanitizationTransformTestBehavior::HangDuringMutation
        | SanitizationTransformTestBehavior::HangDuringSerialization => loop {
            std::thread::park_timeout(Duration::from_secs(60));
        },
        SanitizationTransformTestBehavior::Real => {}
    }
}

fn build_control_results(
    inspection: &SanitizationInspectionReport,
    policy: &SanitizationPolicy,
    mut mutation: MutationEvidence,
) -> Result<Vec<SanitizationControlTransformResult>, crate::job_worker::JobFailure> {
    let mut results = Vec::with_capacity(policy.controls.len());
    for control_id in &policy.controls {
        let detected_before = inspection
            .controls
            .iter()
            .find(|control| control.control_id == *control_id)
            .ok_or_else(|| {
                crate::job_worker::JobFailure::new(
                    "SANITIZATION_TRANSFORM_FAILED",
                    "Inspection control sonucu bulunamadı",
                )
            })?
            .present;
        let removed_count = mutation.counts.remove(control_id).unwrap_or(0);
        if detected_before != (removed_count > 0) {
            return Err(crate::job_worker::JobFailure::new(
                "SANITIZATION_TRANSFORM_FAILED",
                format!("Control mutation inventory ile eşleşmedi: {control_id}"),
            ));
        }
        results.push(SanitizationControlTransformResult {
            control_id: control_id.clone(),
            requested: true,
            detected_before,
            removed_count,
            transform_status: if detected_before {
                SanitizationTransformStatus::RemovedCandidate
            } else {
                SanitizationTransformStatus::NotFound
            },
            evidence: mutation.evidence.remove(control_id).unwrap_or_default(),
            warnings: Vec::new(),
        });
    }
    Ok(results)
}

fn checked_private_output_path(
    job_dir: &Path,
    output_name: &str,
) -> Result<PathBuf, crate::job_worker::JobFailure> {
    let metadata = fs::symlink_metadata(job_dir).map_err(|error| {
        crate::job_worker::JobFailure::new(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            format!("Job directory doğrulanamadı: {error}"),
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(crate::job_worker::JobFailure::new(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Job directory symlink veya directory dışı",
        ));
    }
    if output_name != SANITIZATION_TRANSFORM_OUTPUT_NAME
        || Path::new(output_name).components().count() != 1
    {
        return Err(crate::job_worker::JobFailure::new(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Output adı allowlist dışı",
        ));
    }
    let output = job_dir.join(output_name);
    if fs::symlink_metadata(&output).is_ok() {
        return Err(crate::job_worker::JobFailure::new(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Output path önceden mevcut",
        ));
    }
    Ok(output)
}

pub struct SanitizationTransformCoreParams {
    pub expected_content_generation: u64,
    pub expected_input_digest: String,
    pub expected_inspection_digest: String,
    pub policy: SanitizationPolicy,
    pub timeout: Duration,
}

pub async fn transform_document_sanitization_core(
    job_id: String,
    session_id: String,
    params: SanitizationTransformCoreParams,
    store: &crate::document_session::DocumentSessionStore,
) -> Result<
    SessionJobOutcome<SanitizationTransformReport>,
    crate::document_session::DocumentSessionError,
> {
    transform_document_sanitization_core_inner(
        job_id,
        session_id,
        params,
        store,
        #[cfg(feature = "sanitization-test-support")]
        SanitizationTransformTestBehavior::Real,
    )
    .await
}

/// Internal 8C.4 orchestration. Transform and verification are separate worker
/// processes inside one snapshot-job lifetime; no public command or target
/// publication path exposes the resulting private artifact.
#[allow(clippy::too_many_arguments)]
pub async fn transform_and_verify_document_sanitization_core(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    expected_input_digest: String,
    expected_inspection_digest: String,
    policy: SanitizationPolicy,
    store: &crate::document_session::DocumentSessionStore,
    transform_timeout: Duration,
    verification_timeout: Duration,
    #[cfg(feature = "sanitization-test-support")]
    transform_test_behavior: SanitizationTransformTestBehavior,
    #[cfg(feature = "sanitization-test-support")]
    verification_test_behavior: crate::sanitization_verifier::SanitizationVerificationTestBehavior,
) -> Result<
    SessionJobOutcome<SanitizationVerificationReport>,
    crate::document_session::DocumentSessionError,
> {
    validate_sanitization_policy(&policy).map_err(|error| match error {
        SessionJobOperationError::Failed(failure) => {
            crate::document_session::DocumentSessionError::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(_) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                "Sanitization verification başlamadan iptal edildi",
            )
        }
    })?;
    let parent_policy = policy.clone();
    let parent_input_digest = expected_input_digest.clone();
    let parent_inspection_digest = expected_inspection_digest.clone();
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let transform_result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::SanitizePdf {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        expected_input_digest: expected_input_digest.clone(),
                        expected_inspection_digest: expected_inspection_digest.clone(),
                        output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior: transform_test_behavior,
                    },
                    transform_timeout,
                )?;
                let (transform_report, artifact) = validate_transform_worker_result(
                    &context,
                    transform_result,
                    &parent_policy,
                    &parent_input_digest,
                    &parent_inspection_digest,
                )?;

                // Mandatory parent-side pre-verification gate. The second
                // process is never started for stale/tampered transform bytes.
                let pre_verification_digest = crate::document_io::sha256_file(&artifact)
                    .map_err(|_| {
                        transform_error(
                            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
                            "Pre-verification artifact digest okunamadı",
                        )
                    })?;
                if pre_verification_digest != transform_report.output_digest {
                    return Err(transform_error(
                        "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
                        "Pre-verification artifact digest transform report ile eşleşmedi",
                    ));
                }

                let verification_result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::VerifySanitizedPdf {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        expected_input_digest: expected_input_digest.clone(),
                        expected_transform_digest: pre_verification_digest.clone(),
                        transform_report: transform_report.clone(),
                        output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior: verification_test_behavior,
                    },
                    verification_timeout,
                )?;
                let crate::job_worker::JobResult::SanitizationVerification(report) =
                    verification_result
                else {
                    return Err(transform_error(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Sanitization verifier beklenmeyen result variant döndürdü",
                    ));
                };
                validate_verification_report_for_transform(
                    &report,
                    &transform_report,
                    &parent_policy,
                )?;
                let post_verification_digest = crate::document_io::sha256_file(&artifact)
                    .map_err(|_| {
                        transform_error(
                            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
                            "Post-verification artifact digest okunamadı",
                        )
                    })?;
                if post_verification_digest != transform_report.output_digest
                    || post_verification_digest != report.transform_digest
                {
                    return Err(transform_error(
                        "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
                        "Transform/verifier/actual artifact digest invariantı eşleşmedi",
                    ));
                }
                if !report.is_verified {
                    let checks = [
                        ("full_rewrite", &report.full_rewrite),
                        ("incremental_history_removed", &report.incremental_history_removed),
                        ("object_graph_scan", &report.object_graph_scan),
                        ("removed_payload_scan", &report.removed_payload_scan),
                        ("parser_reopen", &report.parser_reopen),
                        ("pdfium_reopen", &report.pdfium_reopen),
                        ("page_tree_valid", &report.page_tree_valid),
                        ("page_count_valid", &report.page_count_valid),
                        ("page_dimensions_valid", &report.page_dimensions_valid),
                        ("preservation_checks", &report.preservation_checks),
                        ("render_smoke", &report.render_smoke),
                        ("artifact_unchanged", &report.artifact_unchanged),
                    ];
                    let failed_checks = checks
                        .into_iter()
                        .filter(|(_, check)| {
                            check.status
                                != crate::sanitization::SanitizationVerificationStatus::Passed
                        })
                        .map(|(name, _)| name)
                        .chain(
                            report
                                .controls
                                .iter()
                                .filter(|control| {
                                    control.status
                                        != crate::sanitization::SanitizationVerificationStatus::Passed
                                })
                                .map(|control| control.control_id.as_str()),
                        )
                        .collect::<Vec<_>>()
                        .join(",");
                    return Err(transform_error(
                        "SANITIZATION_VERIFICATION_FAILED",
                        format!(
                            "Independent sanitization verification mandatory gate'i geçmedi: {failed_checks}"
                        ),
                    ));
                }
                Ok(report)
            },
        )
        .await
}

async fn transform_document_sanitization_core_inner(
    job_id: String,
    session_id: String,
    params: SanitizationTransformCoreParams,
    store: &crate::document_session::DocumentSessionStore,
    #[cfg(feature = "sanitization-test-support")] test_behavior: SanitizationTransformTestBehavior,
) -> Result<
    SessionJobOutcome<SanitizationTransformReport>,
    crate::document_session::DocumentSessionError,
> {
    let SanitizationTransformCoreParams {
        expected_content_generation,
        expected_input_digest,
        expected_inspection_digest,
        policy,
        timeout,
    } = params;
    validate_sanitization_policy(&policy).map_err(|error| match error {
        SessionJobOperationError::Failed(failure) => {
            crate::document_session::DocumentSessionError::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(_) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                "Policy validation cancelled",
            )
        }
    })?;
    let parent_policy = policy.clone();
    let parent_input_digest = expected_input_digest.clone();
    let parent_inspection_digest = expected_inspection_digest.clone();
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::SanitizePdf {
                        content_generation: context.captured_content_generation,
                        policy,
                        expected_input_digest,
                        expected_inspection_digest,
                        output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior,
                    },
                    timeout,
                )?;
                let (report, _) = validate_transform_worker_result(
                    &context,
                    result,
                    &parent_policy,
                    &parent_input_digest,
                    &parent_inspection_digest,
                )?;
                Ok(report)
            },
        )
        .await
}

pub(crate) fn validate_transform_worker_result(
    context: &crate::document_session::SessionJobContext,
    result: crate::job_worker::JobResult,
    policy: &SanitizationPolicy,
    expected_input_digest: &str,
    expected_inspection_digest: &str,
) -> Result<(SanitizationTransformReport, PathBuf), SessionJobOperationError> {
    let crate::job_worker::JobResult::SanitizationTransform(report) = result else {
        return Err(transform_error(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Sanitization worker beklenmeyen result variant döndürdü",
        ));
    };
    validate_transform_report_for_policy(
        &report,
        policy,
        expected_input_digest,
        expected_inspection_digest,
    )?;
    if report.content_generation != context.captured_content_generation {
        return Err(transform_error(
            "SANITIZATION_STALE_INSPECTION",
            "Transform report content generation eşleşmedi",
        ));
    }
    let job_dir = context.snapshot_path.parent().ok_or_else(|| {
        transform_error(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Job directory bulunamadı",
        )
    })?;
    let artifact_candidate = job_dir.join(&report.output_relative_name);
    let candidate_metadata = fs::symlink_metadata(&artifact_candidate).map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Artifact metadata okunamadı",
        )
    })?;
    if !candidate_metadata.file_type().is_file()
        || candidate_metadata.file_type().is_symlink()
        || candidate_metadata.len() != report.output_size_bytes
    {
        return Err(transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Artifact regular file/size invariantı geçersiz",
        ));
    }
    let artifact = crate::job_worker::validate_output_file(context, &artifact_candidate)?;
    let canonical_job_dir = job_dir.canonicalize().map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Private job directory canonicalize edilemedi",
        )
    })?;
    if artifact != canonical_job_dir.join(SANITIZATION_TRANSFORM_OUTPUT_NAME) {
        return Err(transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Artifact canonical private filename invariantı geçersiz",
        ));
    }
    let digest = crate::document_io::sha256_file(&artifact).map_err(|_| {
        transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Artifact digest okunamadı",
        )
    })?;
    if digest != report.output_digest {
        return Err(transform_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
            "Artifact digest worker report ile eşleşmedi",
        ));
    }
    Ok((report, artifact))
}

/// Test-only observability for the parent validation boundary. It is compiled
/// only with the acceptance-test support feature and is not an IPC surface.
#[cfg(feature = "sanitization-test-support")]
#[doc(hidden)]
pub fn validate_transform_worker_result_for_acceptance(
    context: &crate::document_session::SessionJobContext,
    result: crate::job_worker::JobResult,
    policy: &SanitizationPolicy,
    expected_input_digest: &str,
    expected_inspection_digest: &str,
) -> Result<(SanitizationTransformReport, PathBuf), SessionJobOperationError> {
    validate_transform_worker_result(
        context,
        result,
        policy,
        expected_input_digest,
        expected_inspection_digest,
    )
}

#[cfg(feature = "sanitization-test-support")]
pub async fn transform_document_sanitization_core_with_test_support(
    job_id: String,
    session_id: String,
    params: SanitizationTransformCoreParams,
    store: &crate::document_session::DocumentSessionStore,
    test_behavior: SanitizationTransformTestBehavior,
) -> Result<
    SessionJobOutcome<SanitizationTransformReport>,
    crate::document_session::DocumentSessionError,
> {
    transform_document_sanitization_core_inner(job_id, session_id, params, store, test_behavior)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sanitization::{SanitizationAcknowledgements, SanitizationProfile};
    use lopdf::dictionary;
    use tempfile::tempdir;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization")
            .join(name)
    }

    fn policy(controls: &[&str], acknowledge_signature_invalidation: bool) -> SanitizationPolicy {
        let mut controls: Vec<String> = controls
            .iter()
            .map(|control| (*control).to_string())
            .collect();
        if !controls
            .iter()
            .any(|control| control == "incremental_revisions")
        {
            controls.push("incremental_revisions".to_string());
        }
        SanitizationPolicy {
            schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            profile: SanitizationProfile::CustomSupportedV1,
            controls,
            acknowledgements: SanitizationAcknowledgements {
                acknowledge_signature_invalidation,
            },
            expected_inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
        }
    }

    fn transform(
        name: &str,
        controls: &[&str],
        credential: Option<&str>,
        acknowledge_signature_invalidation: bool,
    ) -> Result<(tempfile::TempDir, SanitizationTransformReport), crate::job_worker::JobFailure>
    {
        let dir = tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.pdf");
        fs::copy(fixture(name), &snapshot).unwrap();
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &snapshot,
            credential,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let expected_inspection_digest = inspection_digest(&inspection).unwrap();
        let input_digest = inspection.input_digest.clone();
        let report = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential,
            content_generation: 7,
            policy: policy(controls, acknowledge_signature_invalidation),
            expected_input_digest: &input_digest,
            expected_inspection_digest: &expected_inspection_digest,
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })?;
        Ok((dir, report))
    }

    fn bytes_contain(bytes: &[u8], marker: &[u8]) -> bool {
        bytes.windows(marker.len()).any(|window| window == marker)
    }

    #[test]
    fn policy_rejects_legacy_unknown_unsupported_and_duplicate_controls() {
        assert!(validate_sanitization_policy(&policy(&["remove_hidden_layers"], false)).is_err());
        assert!(validate_sanitization_policy(&policy(&["optional_content"], false)).is_err());
        assert!(validate_sanitization_policy(&policy(
            &["document_javascript", "document_javascript"],
            false
        ))
        .is_err());
    }

    #[test]
    fn fingerprint_decoded_stream_limits_are_exact_post_decode_and_fail_closed() {
        let mut evidence = MutationEvidence::default();
        evidence
            .account_fingerprint_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES, true)
            .unwrap();
        assert_eq!(
            evidence.decoded_bytes,
            MAX_SANITIZATION_DECODED_STREAM_BYTES
        );

        let error = evidence
            .account_fingerprint_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES + 1, true)
            .unwrap_err();
        assert!(matches!(
            error,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));

        // Strings retain their existing total-payload accounting semantics;
        // they are not reclassified as decoded streams.
        let mut string_payload = MutationEvidence::default();
        string_payload
            .account_fingerprint_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES + 1, false)
            .unwrap();
    }

    #[test]
    fn fingerprint_total_decoded_budget_and_overflow_fail_closed() {
        let mut evidence = MutationEvidence::default();
        assert_eq!(
            MAX_SANITIZATION_TOTAL_DECODED_BYTES % MAX_SANITIZATION_DECODED_STREAM_BYTES,
            0
        );
        for _ in 0..(MAX_SANITIZATION_TOTAL_DECODED_BYTES / MAX_SANITIZATION_DECODED_STREAM_BYTES) {
            evidence
                .account_fingerprint_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES, true)
                .unwrap();
        }
        assert_eq!(evidence.decoded_bytes, MAX_SANITIZATION_TOTAL_DECODED_BYTES);
        let error = evidence.account_fingerprint_bytes(1, true).unwrap_err();
        assert!(matches!(
            error,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));

        let mut overflow = MutationEvidence {
            decoded_bytes: u64::MAX,
            ..MutationEvidence::default()
        };
        let error = overflow.account_fingerprint_bytes(1, false).unwrap_err();
        assert!(matches!(
            error,
            SessionJobOperationError::Failed(ref failure)
                if failure.code == "SANITIZATION_DECODE_LIMIT_EXCEEDED"
        ));
    }

    #[test]
    fn parent_rejects_transform_report_not_exactly_bound_to_request_policy() {
        let (_, report) =
            transform("clean-minimal.pdf", &["document_javascript"], None, false).unwrap();
        let policy = policy(&["document_javascript"], false);
        let input_digest = report.input_digest.clone();
        let inspection_digest = report.inspection_digest.clone();
        validate_transform_report_for_policy(&report, &policy, &input_digest, &inspection_digest)
            .unwrap();

        let mut missing = report.clone();
        missing.controls.pop();
        assert!(validate_transform_report_for_policy(
            &missing,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut extra = report.clone();
        let mut extra_control = extra.controls[0].clone();
        extra_control.control_id = "uri_actions".to_string();
        extra.controls.push(extra_control);
        assert!(validate_transform_report_for_policy(
            &extra,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut wrong_profile = report.clone();
        wrong_profile.profile = SanitizationProfile::VerifiedLimitedV1;
        assert!(validate_transform_report_for_policy(
            &wrong_profile,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut wrong_policy_schema = report.clone();
        wrong_policy_schema.policy_schema_version = "sanitization-policy-v0".to_string();
        assert!(validate_transform_report_for_policy(
            &wrong_policy_schema,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut duplicate = report.clone();
        duplicate.controls.push(duplicate.controls[0].clone());
        assert!(validate_transform_report_for_policy(
            &duplicate,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut wrong_input = report.clone();
        wrong_input.input_digest = "0".repeat(64);
        assert!(validate_transform_report_for_policy(
            &wrong_input,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());

        let mut wrong_inspection = report;
        wrong_inspection.inspection_digest = "1".repeat(64);
        assert!(validate_transform_report_for_policy(
            &wrong_inspection,
            &policy,
            &input_digest,
            &inspection_digest,
        )
        .is_err());
    }

    #[test]
    fn stale_inspection_and_non_allowlisted_output_name_fail_before_artifact() {
        let dir = tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.pdf");
        fs::copy(fixture("clean-minimal.pdf"), &snapshot).unwrap();
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &snapshot,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let error = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential: None,
            content_generation: 1,
            policy: policy(&["incremental_revisions"], false),
            expected_input_digest: &inspection.input_digest,
            expected_inspection_digest: &"0".repeat(64),
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap_err();
        assert_eq!(error.code, "SANITIZATION_STALE_INSPECTION");
        let error = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential: None,
            content_generation: 1,
            policy: policy(&["incremental_revisions"], false),
            expected_input_digest: &inspection.input_digest,
            expected_inspection_digest: &inspection_digest(&inspection).unwrap(),
            output_name: "../escaped.pdf",
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap_err();
        assert_eq!(error.code, "SESSION_JOB_WORKER_PATH_REJECTED");
        assert!(!dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME).exists());
    }

    #[test]
    fn names_mutation_is_branch_specific_and_prunes_payload() {
        let (dir, report) =
            transform("combined-names.pdf", &["document_javascript"], None, false).unwrap();
        let output = dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let document = Document::load(&output).unwrap();
        let names = resolve_dictionary(
            &document,
            document.catalog().unwrap().get(b"Names").unwrap(),
        )
        .unwrap();
        assert!(names.has(b"Dests"));
        assert!(names.has(b"EmbeddedFiles"));
        assert!(!names.has(b"JavaScript"));
        let bytes = fs::read(output).unwrap();
        assert!(!bytes_contain(
            &bytes,
            b"SANITIZATION_COMBINED_NAMES_JS_001"
        ));
        assert!(bytes_contain(
            &bytes,
            b"SANITIZATION_COMBINED_NAMES_EMBEDDED_001"
        ));
        assert_eq!(report.status, "transformed_unverified");
    }

    #[test]
    fn embedded_mutation_preserves_unrelated_name_tree_branches() {
        let (dir, _) = transform("combined-names.pdf", &["embedded_files"], None, false).unwrap();
        let output = dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let document = Document::load(&output).unwrap();
        let names = resolve_dictionary(
            &document,
            document.catalog().unwrap().get(b"Names").unwrap(),
        )
        .unwrap();
        assert!(names.has(b"Dests"));
        assert!(names.has(b"JavaScript"));
        assert!(!names.has(b"EmbeddedFiles"));
        let bytes = fs::read(output).unwrap();
        assert!(bytes_contain(&bytes, b"SANITIZATION_COMBINED_NAMES_JS_001"));
        assert!(!bytes_contain(
            &bytes,
            b"SANITIZATION_COMBINED_NAMES_EMBEDDED_001"
        ));
    }

    #[test]
    fn open_action_destination_is_preserved() {
        let (dir, report) = transform(
            "openaction-destination.pdf",
            &["document_javascript"],
            None,
            false,
        )
        .unwrap();
        let document = Document::load(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        assert!(document.catalog().unwrap().has(b"OpenAction"));
        assert_eq!(
            report.controls[0].transform_status,
            SanitizationTransformStatus::NotFound
        );
    }

    #[test]
    fn targeted_action_types_remove_nested_next_but_local_goto_is_preserved() {
        let (dir, report) = transform(
            "action-chain-network.pdf",
            &[
                "launch_actions",
                "uri_actions",
                "remote_navigation_actions",
                "form_submission_actions",
            ],
            None,
            false,
        )
        .unwrap();
        assert!(report.controls.iter().all(|control| {
            control.control_id == "incremental_revisions"
                || control.transform_status == SanitizationTransformStatus::RemovedCandidate
        }));
        let document = Document::load(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        let mut action_types = Vec::new();
        for object in document.objects.values() {
            if let Ok(dictionary) = object.as_dict() {
                if let Some(name) = dictionary_name(dictionary, b"S") {
                    action_types.push(name);
                }
            }
        }
        assert!(!action_types.iter().any(|name| matches!(
            name.as_slice(),
            b"Launch" | b"URI" | b"GoToR" | b"GoToE" | b"SubmitForm" | b"ImportData"
        )));
    }

    #[test]
    fn local_goto_action_is_preserved_when_remote_actions_are_requested() {
        let dir = tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "OpenAction" => dictionary! {
                "S" => "GoTo",
                "D" => vec![Object::Reference(page_id), Object::Name(b"Fit".to_vec())],
            },
        });
        document.trailer.set("Root", catalog_id);
        document.save(&snapshot).unwrap();
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &snapshot,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential: None,
            content_generation: 1,
            policy: policy(&["remote_navigation_actions"], false),
            expected_input_digest: &inspection.input_digest,
            expected_inspection_digest: &inspection_digest(&inspection).unwrap(),
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap();
        let output = Document::load(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        let open_action = resolve_dictionary(
            &output,
            output.catalog().unwrap().get(b"OpenAction").unwrap(),
        )
        .unwrap();
        assert_eq!(
            dictionary_name(open_action, b"S").as_deref(),
            Some(b"GoTo".as_slice())
        );
    }

    #[test]
    fn form_and_annotation_action_removal_preserves_fields_and_widgets() {
        let (dir, report) = transform(
            "javascript-actions.pdf",
            &[
                "document_javascript",
                "additional_actions",
                "annotation_actions",
                "form_actions",
            ],
            None,
            false,
        )
        .unwrap();
        assert!(report.controls.iter().all(|control| {
            control.control_id == "incremental_revisions"
                || control.transform_status == SanitizationTransformStatus::RemovedCandidate
        }));
        let document = Document::load(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        let catalog = document.catalog().unwrap();
        assert!(catalog.has(b"AcroForm"));
        let field_count = document
            .objects
            .values()
            .filter(|object| {
                object
                    .as_dict()
                    .is_ok_and(|dictionary| dictionary.has(b"FT"))
            })
            .count();
        assert!(field_count > 0);
    }

    #[test]
    fn embedded_owner_edges_and_file_attachment_are_pruned_together() {
        let (dir, report) = transform(
            "embedded-files-af.pdf",
            &[
                "embedded_files",
                "associated_files",
                "file_attachment_annotations",
            ],
            None,
            false,
        )
        .unwrap();
        assert!(report.controls.iter().all(|control| {
            control.control_id == "incremental_revisions"
                || control.transform_status == SanitizationTransformStatus::RemovedCandidate
        }));
        let bytes = fs::read(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        assert!(!bytes_contain(&bytes, b"SANITIZATION_EMBEDDED_PAYLOAD_001"));
        let document = Document::load_mem(&bytes).unwrap();
        assert!(document.objects.values().all(|object| {
            object.as_dict().map_or(true, |dictionary| {
                dictionary_name(dictionary, b"Subtype").as_deref() != Some(b"FileAttachment")
            })
        }));
    }

    #[test]
    fn metadata_scopes_are_targeted_and_full_rewrite_reopens() {
        let (dir, report) = transform(
            "metadata-paths.pdf",
            &[
                "document_info",
                "xmp_metadata",
                "page_metadata",
                "object_metadata",
            ],
            None,
            false,
        )
        .unwrap();
        let output = dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let document = Document::load(&output).unwrap();
        assert!(!document.trailer.has(b"Info"));
        assert!(!document.catalog().unwrap().has(b"Metadata"));
        assert!(!document
            .get_dictionary(document.get_pages()[&1])
            .unwrap()
            .has(b"Metadata"));
        assert!(report.controls.iter().all(|control| {
            control.control_id == "incremental_revisions"
                || control.transform_status == SanitizationTransformStatus::RemovedCandidate
        }));
        assert!(report
            .controls
            .iter()
            .flat_map(|control| &control.evidence)
            .any(|evidence| evidence.payload_sha256.is_some()));
    }

    #[test]
    fn incremental_history_is_not_serialized() {
        let (dir, report) = transform(
            "incremental-revision.pdf",
            &[
                "document_javascript",
                "xmp_metadata",
                "incremental_revisions",
            ],
            None,
            false,
        )
        .unwrap();
        let output = fs::read(dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME)).unwrap();
        let input = fs::read(fixture("incremental-revision.pdf")).unwrap();
        assert!(!output.starts_with(&input));
        assert_eq!(
            output
                .windows(b"%%EOF".len())
                .filter(|window| *window == b"%%EOF")
                .count(),
            1
        );
        assert!(!bytes_contain(&output, b"/Prev"));
        assert!(!bytes_contain(
            &output,
            b"SANITIZATION_OLD_REVISION_PAYLOAD_001"
        ));
        assert!(!bytes_contain(&output, b"SANITIZATION_INCREMENTAL_XMP_001"));
        assert!(report.full_rewrite_evidence.parser_reopen_passed);
    }

    #[test]
    fn object_and_xref_stream_inputs_are_rewritten_and_reopened() {
        for (fixture_name, marker) in [
            (
                "object-xref-stream.pdf",
                b"SANITIZATION_OBJECT_STREAM_PAYLOAD_001".as_slice(),
            ),
            (
                "incremental-xref-stream-chain.pdf",
                b"SANITIZATION_XREF_STREAM_LATEST_REVISION_001".as_slice(),
            ),
        ] {
            let (dir, report) = transform(
                fixture_name,
                &[
                    "xmp_metadata",
                    "page_metadata",
                    "object_metadata",
                    "incremental_revisions",
                ],
                None,
                false,
            )
            .unwrap();
            let output_path = dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
            let output = fs::read(&output_path).unwrap();
            assert!(!bytes_contain(&output, marker));
            assert!(!bytes_contain(
                &output,
                b"SANITIZATION_XREF_STREAM_OLD_REVISION_001"
            ));
            assert!(!bytes_contain(&output, b"/Prev"));
            assert!(Document::load(&output_path).is_ok());
            assert!(report.full_rewrite_evidence.parser_reopen_passed);
        }
    }

    #[test]
    fn unsupported_content_and_encryption_fail_without_artifact() {
        let dir = tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.pdf");
        fs::copy(fixture("unsupported-active-content.pdf"), &snapshot).unwrap();
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &snapshot,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let error = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential: None,
            content_generation: 1,
            policy: policy(&["document_javascript"], false),
            expected_input_digest: &inspection.input_digest,
            expected_inspection_digest: &inspection_digest(&inspection).unwrap(),
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap_err();
        assert_eq!(error.code, "SANITIZATION_UNSUPPORTED_CONTENT");
        assert!(!dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME).exists());

        let encrypted_dir = tempdir().unwrap();
        let encrypted_snapshot = encrypted_dir.path().join("snapshot.pdf");
        fs::copy(fixture("encrypted.pdf"), &encrypted_snapshot).unwrap();
        let encrypted = crate::sanitization_inspection::inspect_snapshot(
            &encrypted_snapshot,
            Some("fixture-password"),
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let error = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: encrypted_dir.path(),
            snapshot_path: &encrypted_snapshot,
            credential: Some("fixture-password"),
            content_generation: 1,
            policy: policy(&["document_info"], false),
            expected_input_digest: &encrypted.input_digest,
            expected_inspection_digest: &inspection_digest(&encrypted).unwrap(),
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap_err();
        assert_eq!(error.code, "SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED");
        assert!(!encrypted_dir
            .path()
            .join(SANITIZATION_TRANSFORM_OUTPUT_NAME)
            .exists());
    }

    #[test]
    fn signed_input_requires_explicit_acknowledgement_and_reports_invalidation() {
        let dir = tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.pdf");
        fs::copy(fixture("signature.pdf"), &snapshot).unwrap();
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &snapshot,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let error = execute_transform(SanitizationTransformExecutionArgs {
            job_dir: dir.path(),
            snapshot_path: &snapshot,
            credential: None,
            content_generation: 1,
            policy: policy(&["incremental_revisions"], false),
            expected_input_digest: &inspection.input_digest,
            expected_inspection_digest: &inspection_digest(&inspection).unwrap(),
            output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior: SanitizationTransformTestBehavior::Real,
        })
        .unwrap_err();
        assert_eq!(
            error.code,
            "SANITIZATION_SIGNATURE_ACKNOWLEDGEMENT_REQUIRED"
        );
        assert!(!dir.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME).exists());

        let (_, report) =
            transform("signature.pdf", &["incremental_revisions"], None, true).unwrap();
        assert!(report.signatures_invalidated);
        assert!(report.certification_lost);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "SANITIZATION_SIGNATURES_INVALIDATED"));
    }
}
