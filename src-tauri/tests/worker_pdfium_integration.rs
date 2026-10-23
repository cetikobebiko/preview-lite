#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Document, Object, Stream, StringFormat};
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        BrokerError, CommandTimeout, PdfiumWorkerBroker, WorkerDocumentKey, WorkerLauncher,
    };
    use preview_alternative_lib::worker::ipc::{WorkerCommand, WorkerRenderTarget, WorkerResult};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn broker() -> PdfiumWorkerBroker {
        PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
            vec![OsString::from("--run-worker")],
        )))
    }

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    fn key(session_id: &str) -> WorkerDocumentKey {
        WorkerDocumentKey {
            session_id: session_id.to_string(),
            content_generation: 1,
        }
    }

    fn unicode_text_fixture() -> (tempfile::TempDir, String) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker-unicode.pdf");
        let lines = [
            "NBSP:A\u{00a0}B ORDER",
            "SOFT:soft\u{00ad}hyphen",
            "COMPOSED:CAFÉ",
            "DECOMPOSED:CAFE\u{0301}",
            "TURKISH:İstanbul I i ı",
            "MULTI-FIRST",
            "MULTI-SECOND",
        ];
        let rotated = "ORDER ORDER ROTATED";
        let mut characters = lines
            .iter()
            .flat_map(|line| line.chars())
            .chain(rotated.chars())
            .collect::<Vec<_>>();
        characters.sort_unstable();
        characters.dedup();
        assert!(characters.len() < 255);
        let codes = characters
            .iter()
            .enumerate()
            .map(|(index, character)| (*character, u8::try_from(index + 1).unwrap()))
            .collect::<HashMap<_, _>>();
        let encode = |text: &str| {
            text.chars()
                .map(|character| *codes.get(&character).unwrap())
                .collect::<Vec<_>>()
        };

        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.new_object_id();
        let mut char_procs = Dictionary::new();
        let mut differences = vec![Object::Integer(1)];
        let mut widths = Vec::new();
        let mut cmap_entries = String::new();
        for (index, character) in characters.iter().enumerate() {
            let code = u8::try_from(index + 1).unwrap();
            let name = format!("g{code:02x}");
            differences.push(Object::Name(name.as_bytes().to_vec()));
            widths.push(Object::Integer(500));
            let glyph = document.add_object(Stream::new(
                dictionary! {},
                b"500 0 0 0 500 700 d1 40 0 420 600 re f".to_vec(),
            ));
            char_procs.set(name.as_bytes(), Object::Reference(glyph));
            let unicode = character
                .encode_utf16(&mut [0u16; 2])
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect::<String>();
            cmap_entries.push_str(&format!("<{code:02X}> <{unicode}>\n"));
        }
        let cmap = format!(
            "/CIDInit /ProcSet findresource begin\n\
             12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Test) /Ordering (Unicode) /Supplement 0 >> def\n\
             /CMapName /WorkerUnicode def\n/CMapType 2 def\n\
             1 begincodespacerange\n<01> <FF>\nendcodespacerange\n\
             {} beginbfchar\n{}endbfchar\n\
             endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
            characters.len(),
            cmap_entries
        );
        let to_unicode_id = document.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let encoding_id = document.add_object(dictionary! {
            "Type" => "Encoding",
            "Differences" => differences,
        });
        document.objects.insert(
            font_id,
            Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type3",
                "Name" => "F1",
                "FontBBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
                "FontMatrix" => vec![0.001.into(), 0.into(), 0.into(), 0.001.into(), 0.into(), 0.into()],
                "CharProcs" => char_procs,
                "Encoding" => encoding_id,
                "FirstChar" => 1,
                "LastChar" => i64::try_from(characters.len()).unwrap(),
                "Widths" => widths,
                "Resources" => Dictionary::new(),
                "ToUnicode" => to_unicode_id,
            }),
        );
        let resources = dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        };
        let mut page_ids = Vec::new();
        for (page_number, page_lines) in [
            (0usize, lines.as_slice()),
            (1usize, std::slice::from_ref(&rotated)),
        ] {
            let mut operations = Vec::new();
            for (line_index, line) in page_lines.iter().enumerate() {
                operations.extend([
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 18.into()]),
                    Operation::new(
                        "Td",
                        vec![
                            30.into(),
                            (360 - i64::try_from(line_index).unwrap() * 40).into(),
                        ],
                    ),
                    Operation::new(
                        "Tj",
                        vec![Object::String(encode(line), StringFormat::Hexadecimal)],
                    ),
                    Operation::new("ET", vec![]),
                ]);
            }
            let content_id = document.add_object(Stream::new(
                dictionary! {},
                Content { operations }.encode().unwrap(),
            ));
            let mut page = dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 500.into(), 400.into()],
                "Resources" => resources.clone(),
                "Contents" => content_id,
            };
            if page_number == 1 {
                page.set("Rotate", 90);
            }
            page_ids.push(document.add_object(page));
        }
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => i64::try_from(page_ids.len()).unwrap(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        (directory, path.to_string_lossy().to_string())
    }

    #[tokio::test]
    async fn worker_preserves_render_thumbnail_text_and_search_contracts() {
        let broker = broker();
        let key = key("worker-parity");
        let admission = broker.admit_read(&key, None).await.unwrap();
        let opened = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("reorder-rich.pdf"),
                None,
                None,
            )
            .await;
        let (_document, page_count, protected) = match opened {
            Ok(opened) => opened,
            Err(error) => panic!(
                "worker open failed: {error:?}; stderr={}",
                broker.last_worker_stderr_for_test().await
            ),
        };
        assert_eq!(page_count, 3);
        assert!(!protected);

        let outline = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetOutline { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(outline, WorkerResult::Outline(nodes) if nodes.is_empty()));

        let metadata = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetMetadata { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(metadata, WorkerResult::Metadata(metadata) if metadata.title.is_empty()));

        let annotations = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageAnnotations {
                    document_id,
                    page_index: 2,
                },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            annotations,
            WorkerResult::PageAnnotations(items)
                if items.len() == 1
                    && items[0].contents.as_deref() == Some("Reorder fixture annotation")
        ));

        let pdf_ua = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::AccessibilityAudit(report)
                if !report.document_facts.has_structure_tree
        ));

        let structure = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetStructureTree { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(
            matches!(structure, WorkerResult::AccessibilityStructure(inspection) if inspection.root.is_none())
        );

        for rotation in [0, 90, 180, 270] {
            let result = broker
                .command_for_document(
                    &admission,
                    &key,
                    |document_id| WorkerCommand::RenderPage {
                        document_id,
                        page_index: 0,
                        target: WorkerRenderTarget::Scale(1.0),
                        rotation,
                    },
                    CommandTimeout::RenderPage,
                    None,
                )
                .await
                .unwrap();
            let WorkerResult::PageRendered {
                width,
                height,
                data,
            } = result
            else {
                panic!("unexpected render response");
            };
            assert_eq!(data.len(), width as usize * height as usize * 4);
            if matches!(rotation, 90 | 270) {
                assert!(width > height);
            } else {
                assert!(height > width);
            }
        }

        let thumbnail = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderThumbnail {
                    document_id,
                    page_index: 0,
                    target_width: 180,
                },
                CommandTimeout::RenderThumbnail,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::PageRendered {
            width,
            height,
            data,
        } = thumbnail
        else {
            panic!("unexpected thumbnail response");
        };
        assert_eq!(width, 180);
        assert_eq!(data.len(), width as usize * height as usize * 4);

        let layout = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageTextLayout {
                    document_id,
                    page_index: 0,
                },
                CommandTimeout::TextLayout,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::TextLayout(layout) = layout else {
            panic!("unexpected text-layout response");
        };
        assert!(layout.iter().any(|item| item.text.contains("PAGE-ONE")));
        assert!(layout.iter().all(|item| {
            [item.x, item.y, item.width, item.height]
                .iter()
                .all(|value| value.is_finite())
        }));

        let search = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "page-one".to_string(),
                    max_results: 100,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::Search(search) = search else {
            panic!("unexpected search response");
        };
        assert_eq!(search.total_matches, 1);
        assert_eq!(search.matches[0].page_index, 0);
        assert!(!search.matches[0].rects.is_empty());

        let ordered = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "page-".to_string(),
                    max_results: 100,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::Search(ordered) = ordered else {
            panic!("unexpected ordered search response");
        };
        assert_eq!(ordered.total_matches, 3);
        assert_eq!(
            ordered
                .matches
                .iter()
                .map(|item| (item.global_match_index, item.page_index))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 1), (2, 2)]
        );

        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);

        let (_unicode_directory, unicode_path) = unicode_text_fixture();
        let unicode_key = WorkerDocumentKey {
            session_id: "worker-unicode-parity".to_string(),
            content_generation: 1,
        };
        let unicode_admission = broker.admit_read(&unicode_key, None).await.unwrap();
        broker
            .ensure_document(
                &unicode_admission,
                unicode_key.clone(),
                unicode_path,
                None,
                None,
            )
            .await
            .unwrap();
        for (query, total_matches, pages) in [
            ("A B", 1, vec![0]),
            ("softhyphen", 1, vec![0]),
            ("café", 2, vec![0, 0]),
            ("istanbul", 1, vec![0]),
            ("multi-first", 1, vec![0]),
            ("order", 3, vec![0, 1, 1]),
        ] {
            let result = broker
                .command_for_document(
                    &unicode_admission,
                    &unicode_key,
                    |document_id| WorkerCommand::SearchText {
                        document_id,
                        query: query.to_string(),
                        max_results: 100,
                    },
                    CommandTimeout::Search,
                    None,
                )
                .await
                .unwrap();
            let WorkerResult::Search(result) = result else {
                panic!("unexpected Unicode search response for {query}");
            };
            assert_eq!(result.total_matches, total_matches, "query={query}");
            assert_eq!(
                result
                    .matches
                    .iter()
                    .map(|item| item.page_index)
                    .collect::<Vec<_>>(),
                pages,
                "query={query}"
            );
            assert_eq!(
                result
                    .matches
                    .iter()
                    .map(|item| item.global_match_index)
                    .collect::<Vec<_>>(),
                (0..result.matches.len()).collect::<Vec<_>>(),
                "query={query}"
            );
        }
        let rotated_layout = broker
            .command_for_document(
                &unicode_admission,
                &unicode_key,
                |document_id| WorkerCommand::GetPageTextLayout {
                    document_id,
                    page_index: 1,
                },
                CommandTimeout::TextLayout,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::TextLayout(rotated_layout) = rotated_layout else {
            panic!("unexpected rotated text layout response");
        };
        assert!(rotated_layout
            .iter()
            .any(|item| item.text.contains("ROTATED")));
        drop(unicode_admission);
        let unicode_exclusive = broker
            .admit_exclusive(&unicode_key.session_id)
            .await
            .unwrap();
        broker
            .invalidate_document(&unicode_exclusive, &unicode_key)
            .await
            .unwrap();
        drop(unicode_exclusive);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn encrypted_open_has_no_partial_document_on_failed_authentication() {
        let broker = broker();
        let key = key("encrypted-worker");
        let admission = broker.admit_read(&key, None).await.unwrap();
        let required = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("encrypted.pdf"),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(required.code, "DOCUMENT_SESSION_PASSWORD_REQUIRED");

        let wrong = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("encrypted.pdf"),
                Some("wrong-password".to_string()),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(wrong.code, "DOCUMENT_SESSION_INVALID_PASSWORD");

        let (_document, page_count, protected) = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("encrypted.pdf"),
                Some("fixture-password".to_string()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(page_count, 1);
        assert!(protected);
        let metadata = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetMetadata { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(metadata, WorkerResult::Metadata(metadata) if metadata.title.is_empty()));
        let pdf_ua = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::AccessibilityAudit(report) if !report.document_facts.has_structure_tree
        ));
        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn malformed_structure_is_a_successful_bounded_audit_and_close_cleans_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("malformed-structure.pdf");
        let mut document = Document::load(fixture("minimal.pdf")).unwrap();
        let structure_id = document.add_object(dictionary! {
            "Type" => "StructTreeRoot",
            "K" => Object::Reference((999, 0)),
        });
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        let catalog = document
            .get_object_mut(catalog_id)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        catalog.set("StructTreeRoot", structure_id);
        catalog.set("MarkInfo", dictionary! { "Marked" => true });
        document.save(&path).unwrap();

        let broker = broker();
        let key = key("malformed-structure-audit");
        let admission = broker.admit_read(&key, None).await.unwrap();
        broker
            .ensure_document(
                &admission,
                key.clone(),
                path.to_string_lossy().to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        let audit = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            audit,
            WorkerResult::AccessibilityAudit(report)
                if report.checks.iter().any(|check|
                    check.id == "structure.integrity"
                        && check.result == preview_alternative_lib::a11y::AccessibilityCheckResult::Fail)
        ));
        let structure = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetStructureTree { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            structure,
            WorkerResult::AccessibilityStructure(inspection)
                if inspection.root.as_ref().unwrap().children[0].association_status == "unresolved"
        ));

        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);
        let read_after_close = broker.admit_read(&key, None).await.unwrap();
        let closed = broker
            .command_for_document(
                &read_after_close,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(closed.code, "PDFIUM_WORKER_PROTOCOL_ERROR");
        drop(read_after_close);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn interactive_document_reads_do_not_reopen_the_source_path() {
        let directory = tempfile::tempdir().unwrap();
        let copied_path = directory.path().join("worker-owned-document.pdf");
        std::fs::copy(fixture("reorder-with-metadata.pdf"), &copied_path).unwrap();

        let broker = broker();
        let key = key("worker-owned-interactive-reads");
        let admission = broker.admit_read(&key, None).await.unwrap();
        broker
            .ensure_document(
                &admission,
                key.clone(),
                copied_path.to_string_lossy().to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        std::fs::remove_file(&copied_path).unwrap();

        let metadata = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetMetadata { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            metadata,
            WorkerResult::Metadata(metadata) if metadata.title == "Must be rejected"
        ));

        let annotations = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageAnnotations {
                    document_id,
                    page_index: 2,
                },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            annotations,
            WorkerResult::PageAnnotations(items)
                if items.len() == 1
                    && items[0].contents.as_deref() == Some("Reorder fixture annotation")
        ));

        let pdf_ua = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::AccessibilityAudit(report) if !report.document_facts.has_structure_tree
        ));

        let outline = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetOutline { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(outline, WorkerResult::Outline(nodes) if nodes.is_empty()));

        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);

        let outline_path = directory.path().join("worker-owned-outline.pdf");
        std::fs::copy(fixture("reorder-with-outline.pdf"), &outline_path).unwrap();
        let outline_key = WorkerDocumentKey {
            session_id: "worker-owned-outline-read".to_string(),
            content_generation: 1,
        };
        let outline_admission = broker.admit_read(&outline_key, None).await.unwrap();
        broker
            .ensure_document(
                &outline_admission,
                outline_key.clone(),
                outline_path.to_string_lossy().to_string(),
                None,
                None,
            )
            .await
            .unwrap();
        std::fs::remove_file(&outline_path).unwrap();
        let outline = broker
            .command_for_document(
                &outline_admission,
                &outline_key,
                |document_id| WorkerCommand::GetOutline { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            outline,
            WorkerResult::Outline(nodes)
                if nodes.len() == 1 && nodes[0].title == "Must be rejected"
        ));
        drop(outline_admission);
        let outline_exclusive = broker
            .admit_exclusive(&outline_key.session_id)
            .await
            .unwrap();
        broker
            .invalidate_document(&outline_exclusive, &outline_key)
            .await
            .unwrap();
        drop(outline_exclusive);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn largest_practical_allowed_render_survives_worker_resource_envelope() {
        let broker = broker();
        let key = key("worker-resource-envelope");
        let admission = broker.admit_read(&key, None).await.unwrap();
        broker
            .ensure_document(&admission, key.clone(), fixture("minimal.pdf"), None, None)
            .await
            .unwrap();
        let rendered = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 0,
                    target: WorkerRenderTarget::TargetWidth(7_000),
                    rotation: 0,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::PageRendered {
            width,
            height,
            data,
        } = rendered
        else {
            panic!("unexpected resource-envelope render response");
        };
        assert_eq!(width, 7_000);
        assert_eq!(data.len(), width as usize * height as usize * 4);
        assert!(data.len() > 240 * 1024 * 1024);
        assert!(
            data.len() <= preview_alternative_lib::pdf_engine::render_service::MAX_RENDER_BYTES
        );
        assert!(broker.worker_epoch().await.is_some());

        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_intrinsic_rotation_characterization_in_viewer_worker() {
        let broker = broker();
        let key = key("worker-intrinsic-rotation-characterization");
        let admission = broker.admit_read(&key, None).await.unwrap();

        broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("reorder-rich.pdf"),
                None,
                None,
            )
            .await
            .unwrap();

        // Page 0: MediaBox 300x400, Intrinsic Rotation 0°
        let r0_v0 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 0,
                    target: WorkerRenderTarget::TargetWidth(300),
                    rotation: 0,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::PageRendered {
            width: w0_v0,
            height: h0_v0,
            ..
        } = r0_v0
        else {
            panic!("expected page 0 render");
        };
        assert_eq!(w0_v0, 300);
        assert_eq!(h0_v0, 400);

        let r0_v90 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 0,
                    target: WorkerRenderTarget::TargetWidth(300),
                    rotation: 90,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::PageRendered {
            width: w0_v90,
            height: h0_v90,
            ..
        } = r0_v90
        else {
            panic!("expected page 0 rotated render");
        };
        assert_eq!(w0_v90, 400);
        assert_eq!(h0_v90, 300);

        // Page 1: MediaBox 400x500, Intrinsic Rotation 90° (effective display width 500, height 400 at viewer rot 0)
        let r1_v0 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 1,
                    target: WorkerRenderTarget::TargetWidth(400),
                    rotation: 0,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::PageRendered {
            width: w1_v0,
            height: h1_v0,
            ..
        } = r1_v0
        else {
            panic!("expected page 1 render");
        };
        assert_eq!(w1_v0, 400);
        assert_eq!(h1_v0, 320);

        let r1_v90 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 1,
                    target: WorkerRenderTarget::TargetWidth(400),
                    rotation: 90,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::PageRendered {
            width: w1_v90,
            height: h1_v90,
            ..
        } = r1_v90
        else {
            panic!("expected page 1 rotated render");
        };
        assert_eq!(w1_v90, 320);
        assert_eq!(h1_v90, 400);

        // Search text on page 0 (0° rotation) and page 1 (90° rotation) to characterize search rect alignment
        let search_res_p0 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "page-one".to_string(),
                    max_results: 100,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::Search(search0) = search_res_p0 else {
            panic!("expected search results for page 0");
        };
        assert_eq!(search0.total_matches, 1);
        let m0 = &search0.matches[0];
        assert_eq!(m0.page_index, 0);
        assert!(!m0.rects.is_empty());
        let r0 = &m0.rects[0];
        assert!(r0.x >= 0.0 && r0.x <= 1.0);
        assert!(r0.y >= 0.0 && r0.y <= 1.0);
        assert!(r0.width > 0.0 && r0.width <= 1.0);
        assert!(r0.height > 0.0 && r0.height <= 1.0);

        let search_res_p1 = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "page-two".to_string(),
                    max_results: 100,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();

        let WorkerResult::Search(search1) = search_res_p1 else {
            panic!("expected search results for page 1");
        };
        assert_eq!(search1.total_matches, 1);
        let m1 = &search1.matches[0];
        assert_eq!(m1.page_index, 1);
        assert!(!m1.rects.is_empty());
        let r1 = &m1.rects[0];
        // Characterization proof: Page 1 has 90° intrinsic rotation.
        // Worker search_rects calculates raw character bounds without applying intrinsic rotation transformation,
        // producing unaligned coordinates (y < 0.0). This behavior is documented for 8A-5 alignment.
        assert!(r1.width > 0.0 && r1.height > 0.0);
        assert!(r1.y < 0.0 || r1.y > 1.0);

        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn release_smoke_document_opens_with_expected_page_count_and_text_tokens() {
        let broker = broker();
        let key = key("release-smoke-integration");
        let admission = broker.admit_read(&key, None).await.unwrap();
        let (_document, page_count, protected) = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("release-smoke.pdf"),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(page_count, 2);
        assert!(!protected);

        let page_count_result = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageCount { document_id },
                CommandTimeout::PageCount,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(page_count_result, WorkerResult::PageCount(2)));

        let metadata = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetMetadata { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(metadata, WorkerResult::Metadata(_)));

        let search_alpha = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "RELEASE_SMOKE_ALPHA_TOKEN".to_string(),
                    max_results: 10,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::Search(results_alpha) = search_alpha else {
            panic!("expected search results for alpha token");
        };
        assert_eq!(results_alpha.total_matches, 1);
        assert_eq!(results_alpha.matches[0].page_index, 0);

        let search_beta = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "RELEASE_SMOKE_BETA_TOKEN".to_string(),
                    max_results: 10,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::Search(results_beta) = search_beta else {
            panic!("expected search results for beta token");
        };
        assert_eq!(results_beta.total_matches, 1);
        assert_eq!(results_beta.matches[0].page_index, 1);

        drop(admission);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn large_page_dimensions_document_opens_with_bounded_geometry() {
        let broker = broker();
        let key = key("large-page-dimensions-integration");
        let admission = broker.admit_read(&key, None).await.unwrap();
        let (_document, page_count, protected) = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture("large-page-dimensions.pdf"),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(page_count, 1);
        assert!(!protected);

        let search = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: "LARGE PAGE DIMENSIONS".to_string(),
                    max_results: 10,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::Search(results) = search else {
            panic!("expected search results for large page text");
        };
        assert!(results.total_matches >= 1);

        let audit = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(audit, WorkerResult::AccessibilityAudit(_)));

        drop(admission);
        broker.shutdown().await.unwrap();
    }

    struct ParityLease {
        admission: preview_alternative_lib::pdf_engine::subprocess_manager::WorkerAdmissionPermit,
        key: WorkerDocumentKey,
        page_count: u32,
    }

    impl preview_alternative_lib::pdf_engine::subprocess_manager::WorkerReadLease for ParityLease {
        fn admission(
            &self,
        ) -> &preview_alternative_lib::pdf_engine::subprocess_manager::WorkerAdmissionPermit
        {
            &self.admission
        }

        fn document_key(&self) -> &WorkerDocumentKey {
            &self.key
        }

        fn page_count(&self) -> u32 {
            self.page_count
        }
    }

    async fn whole_document_search(
        broker: &PdfiumWorkerBroker,
        key: &WorkerDocumentKey,
        path: &str,
        query: &str,
        max_results: u32,
    ) -> preview_alternative_lib::SearchResult {
        // Each step names itself: a flaky failure must say which broker call broke.
        let context = |step: &str, error: BrokerError| -> ! {
            panic!("whole search {step} failed for query={query:?} limit={max_results}: {error:?}")
        };
        let admission = broker
            .admit_read(key, None)
            .await
            .unwrap_or_else(|error| context("admission", error));
        broker
            .ensure_document(&admission, key.clone(), path.to_string(), None, None)
            .await
            .unwrap_or_else(|error| context("open", error));
        let result = broker
            .command_for_document(
                &admission,
                key,
                |document_id| WorkerCommand::SearchText {
                    document_id,
                    query: query.to_string(),
                    max_results,
                },
                CommandTimeout::Search,
                None,
            )
            .await
            .unwrap_or_else(|error| context("command", error));
        let WorkerResult::Search(result) = result else {
            panic!("unexpected whole-document search response");
        };
        result
    }

    async fn chunked_document_search(
        broker: &PdfiumWorkerBroker,
        key: &WorkerDocumentKey,
        path: &str,
        query: &str,
        max_results: usize,
    ) -> preview_alternative_lib::SearchResult {
        let cancellation = tokio_util::sync::CancellationToken::new();
        broker
            .search_text_in_chunks(
                || {
                    let broker = broker.clone();
                    let key = key.clone();
                    let path = path.to_string();
                    async move {
                        let admission = broker.admit_read(&key, None).await?;
                        let (_, page_count, _) = broker
                            .ensure_document(&admission, key.clone(), path, None, None)
                            .await?;
                        Ok::<_, preview_alternative_lib::pdf_engine::subprocess_manager::BrokerError>(
                            ParityLease {
                                admission,
                                key,
                                page_count,
                            },
                        )
                    }
                },
                query,
                preview_alternative_lib::pdf_engine::subprocess_manager::SearchChunkPolicy {
                    // One page per chunk exercises every chunk boundary.
                    max_pages: 1,
                    time_budget_ms: 10_000,
                    max_results,
                },
                &cancellation,
            )
            .await
            .unwrap_or_else(|error| {
                panic!("chunked search failed for query={query:?} limit={max_results}: {error:?}")
            })
    }

    /// The first difference between two search results, or None when equal.
    /// Two serialized results can be thousands of lines; this names the spot.
    fn first_search_difference(
        chunked: &preview_alternative_lib::SearchResult,
        whole: &preview_alternative_lib::SearchResult,
    ) -> Option<String> {
        if chunked.total_matches != whole.total_matches {
            return Some(format!(
                "total_matches chunked={} whole={}",
                chunked.total_matches, whole.total_matches
            ));
        }
        if chunked.matches.len() != whole.matches.len() {
            return Some(format!(
                "returned matches chunked={} whole={}",
                chunked.matches.len(),
                whole.matches.len()
            ));
        }
        chunked
            .matches
            .iter()
            .zip(&whole.matches)
            .enumerate()
            .find_map(|(index, (left, right))| {
                let left = serde_json::to_value(left).unwrap();
                let right = serde_json::to_value(right).unwrap();
                (left != right).then(|| format!("match {index}: chunked={left} whole={right}"))
            })
            .or_else(|| {
                (serde_json::to_value(chunked).unwrap() != serde_json::to_value(whole).unwrap())
                    .then(|| "results differ outside matches and totals".to_string())
            })
    }

    #[test]
    fn first_search_difference_names_the_first_mismatch() {
        let found = |index: usize, page: u32, snippet: &str| preview_alternative_lib::SearchMatch {
            global_match_index: index,
            page_index: page,
            rects: Vec::new(),
            snippet: snippet.to_string(),
        };
        let result = |matches: Vec<preview_alternative_lib::SearchMatch>, total: usize| {
            preview_alternative_lib::SearchResult {
                matches,
                is_cancelled: false,
                total_matches: total,
            }
        };
        let whole = result(vec![found(0, 0, "a"), found(1, 2, "b")], 5);

        assert_eq!(first_search_difference(&whole.clone(), &whole), None);
        assert_eq!(
            first_search_difference(&result(whole.matches.clone(), 4), &whole).as_deref(),
            Some("total_matches chunked=4 whole=5")
        );
        let moved = result(vec![found(0, 0, "a"), found(1, 3, "b")], 5);
        let difference = first_search_difference(&moved, &whole).unwrap();
        assert!(difference.starts_with("match 1: "), "{difference}");
        assert!(
            difference.contains("\"pageIndex\":3") || difference.contains("\"page_index\":3"),
            "{difference}"
        );
    }

    #[tokio::test]
    async fn chunked_search_is_identical_to_whole_document_search() {
        let broker = broker();
        let (_unicode_directory, unicode_path) = unicode_text_fixture();
        let cases = [
            (
                fixture("three-distinct-pages.pdf"),
                vec!["page-", "page-one", "e"],
            ),
            (
                fixture("ai-text-rich.pdf"),
                vec!["the", "e", "zzzz-not-present"],
            ),
            (fixture("release-smoke.pdf"), vec!["a", "release"]),
            (unicode_path, vec!["order", "café", "istanbul", "A B"]),
        ];
        let mut compared_matches = 0usize;
        for (index, (path, queries)) in cases.iter().enumerate() {
            let key = WorkerDocumentKey {
                session_id: format!("chunk-parity-{index}"),
                content_generation: 1,
            };
            for query in queries {
                for limit in [10_000u32, 2] {
                    let whole = whole_document_search(&broker, &key, path, query, limit).await;
                    let chunked = chunked_document_search(
                        &broker,
                        &key,
                        path,
                        query,
                        usize::try_from(limit).unwrap(),
                    )
                    .await;
                    if let Some(difference) = first_search_difference(&chunked, &whole) {
                        panic!("path={path} query={query} limit={limit}: {difference}");
                    }
                    compared_matches += whole.matches.len();
                }
            }
        }
        // Guard against a vacuous comparison of empty results.
        assert!(
            compared_matches > 10,
            "only {compared_matches} matches compared"
        );
        broker.shutdown().await.unwrap();
    }
    fn page_geometry_contract() -> serde_json::Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/contracts/page-render-geometry-v1.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn json_numbers(value: &serde_json::Value) -> Vec<f64> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|number| number.as_f64().unwrap())
            .collect()
    }

    fn page_geometry_fixture(contract: &serde_json::Value) -> (tempfile::TempDir, String) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("page-geometry.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();
        for page in contract["pages"].as_array().unwrap() {
            let to_box = |value: &serde_json::Value| {
                json_numbers(value)
                    .into_iter()
                    .map(|number| Object::Real(number as f32))
                    .collect::<Vec<_>>()
            };
            let mut dictionary = dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => to_box(&page["mediaBox"]),
            };
            if !page["cropBox"].is_null() {
                dictionary.set("CropBox", to_box(&page["cropBox"]));
            }
            let rotate = page["rotate"].as_i64().unwrap();
            if rotate != 0 {
                dictionary.set("Rotate", rotate);
            }
            page_ids.push(document.add_object(dictionary));
        }
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => i64::try_from(page_ids.len()).unwrap(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        (directory, path.to_string_lossy().to_string())
    }

    /// Pins the page sizes and real PDFium bitmap sizes that the frontend
    /// placeholder-height estimate (src/utils/pageGeometry.js) is tested against.
    #[tokio::test]
    async fn page_sizes_and_render_sizes_match_geometry_contract() {
        let contract = page_geometry_contract();
        let (_directory, path) = page_geometry_fixture(&contract);
        let pages = contract["pages"].as_array().unwrap();
        let broker = broker();
        let key = key("page-geometry-contract");
        let admission = broker.admit_read(&key, None).await.unwrap();
        let (_document, page_count, _protected) = broker
            .ensure_document(&admission, key.clone(), path, None, None)
            .await
            .unwrap();
        assert_eq!(page_count as usize, pages.len());

        let sizes = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageSizes { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        let WorkerResult::PageSizes(sizes) = sizes else {
            panic!("expected page sizes");
        };
        assert_eq!(sizes.len(), pages.len());
        for (size, page) in sizes.iter().zip(pages) {
            let id = page["id"].as_str().unwrap();
            assert!(
                (f64::from(size.width_pt) - page["widthPt"].as_f64().unwrap()).abs() < 0.01
                    && (f64::from(size.height_pt) - page["heightPt"].as_f64().unwrap()).abs()
                        < 0.01,
                "{id}: worker reported {size:?}"
            );
        }

        let mut actual = Vec::new();
        for (page_index, page) in pages.iter().enumerate() {
            for view_scale in json_numbers(&contract["viewScales"]) {
                for dpr in json_numbers(&contract["devicePixelRatios"]) {
                    for rotation in json_numbers(&contract["rotations"]) {
                        let rendered = broker
                            .command_for_document(
                                &admission,
                                &key,
                                |document_id| WorkerCommand::RenderPage {
                                    document_id,
                                    page_index: u32::try_from(page_index).unwrap(),
                                    target: WorkerRenderTarget::Scale(view_scale * dpr),
                                    rotation: rotation as i32,
                                },
                                CommandTimeout::RenderPage,
                                None,
                            )
                            .await
                            .unwrap();
                        let WorkerResult::PageRendered { width, height, .. } = rendered else {
                            panic!("expected rendered page");
                        };
                        actual.push(serde_json::json!({
                            "page": page["id"],
                            "viewScale": view_scale,
                            "devicePixelRatio": dpr,
                            "rotation": rotation,
                            "bitmapWidth": width,
                            "bitmapHeight": height,
                        }));
                    }
                }
            }
        }
        assert_eq!(
            serde_json::Value::Array(actual.clone()),
            contract["renders"],
            "PDFium render sizes changed; actual renders:\n{}",
            serde_json::to_string(&actual).unwrap()
        );

        drop(admission);
        broker.shutdown().await.unwrap();
    }

    /// Real PDFium: the shading page (index 2) of `heavy-page.pdf` cannot be
    /// rasterized inside the 2 s class used here (production classes are
    /// 10-20 s; see `heavy_page_command_timings`). Two timeouts on different
    /// surfaces of that page quarantine only the page. The nested-form page
    /// (index 1) is not used: its parse allocates gigabytes.
    #[tokio::test]
    async fn heavy_page_timeouts_quarantine_the_page_not_the_document() {
        let path = fixture("heavy-page.pdf");
        let broker = broker();
        let key = key("heavy-page-quarantine");
        let render = |page_index| {
            Box::new(move |document_id| WorkerCommand::RenderPage {
                document_id,
                page_index,
                target: WorkerRenderTarget::Scale(0.5),
                rotation: 0,
            }) as Box<dyn FnOnce(u64) -> WorkerCommand + Send>
        };
        let run = |command: Box<dyn FnOnce(u64) -> WorkerCommand + Send>| {
            let broker = broker.clone();
            let key = key.clone();
            let path = path.clone();
            async move {
                let admission = broker.admit_read(&key, None).await.unwrap();
                broker
                    .ensure_document(&admission, key.clone(), path, None, None)
                    .await
                    .map_err(|error| error.code)?;
                broker
                    .command_for_document(&admission, &key, command, CommandTimeout::Ping, None)
                    .await
                    .map_err(|error| error.code)
            }
        };

        assert_eq!(run(render(2)).await.unwrap_err(), "PDFIUM_WORKER_TIMEOUT");
        assert_eq!(
            run(Box::new(|document_id| WorkerCommand::RenderThumbnail {
                document_id,
                page_index: 2,
                target_width: 150,
            }))
            .await
            .unwrap_err(),
            "PDFIUM_WORKER_TIMEOUT"
        );
        assert_eq!(run(render(2)).await.unwrap_err(), "PDFIUM_PAGE_QUARANTINED");

        // From here on nothing may kill the worker: the quarantined page is
        // rejected before it is sent and the light pages render normally.
        let epoch = broker.worker_epoch().await;
        assert!(epoch.is_some());
        let restarts = broker.restart_attempts_for_test().await;
        assert!(matches!(
            run(render(0)).await,
            Ok(WorkerResult::PageRendered { .. })
        ));
        assert!(matches!(
            run(render(3)).await,
            Ok(WorkerResult::PageRendered { .. })
        ));
        assert_eq!(
            run(Box::new(|document_id| WorkerCommand::GetPageTextLayout {
                document_id,
                page_index: 2,
            }))
            .await
            .unwrap_err(),
            "PDFIUM_PAGE_QUARANTINED"
        );
        assert_eq!(broker.worker_epoch().await, epoch);
        assert_eq!(broker.restart_attempts_for_test().await, restarts);
        broker.shutdown().await.unwrap();
    }

    async fn timed_heavy_page_command(
        broker: &PdfiumWorkerBroker,
        key: &WorkerDocumentKey,
        path: &str,
        page_index: u32,
        command: &str,
        timeout: CommandTimeout,
    ) -> (std::time::Duration, String) {
        let admission = broker.admit_read(key, None).await.unwrap();
        if let Err(error) = broker
            .ensure_document(&admission, key.clone(), path.to_string(), None, None)
            .await
        {
            return (std::time::Duration::ZERO, error.code.to_string());
        }
        let started = std::time::Instant::now();
        let result = broker
            .command_for_document(
                &admission,
                key,
                |document_id| match command {
                    "render-page" => WorkerCommand::RenderPage {
                        document_id,
                        page_index,
                        target: WorkerRenderTarget::Scale(1.0),
                        rotation: 0,
                    },
                    "render-thumbnail" => WorkerCommand::RenderThumbnail {
                        document_id,
                        page_index,
                        target_width: 150,
                    },
                    _ => WorkerCommand::GetPageTextLayout {
                        document_id,
                        page_index,
                    },
                },
                timeout,
                None,
            )
            .await;
        let outcome = match result {
            Ok(_) => "ok".to_string(),
            Err(error) => error.code.to_string(),
        };
        (started.elapsed(), outcome)
    }

    /// Real PDFium: page 1 of `heavy-page.pdf` grows its footprint at about
    /// 1.3 GB/s until the 20 s render deadline (25 GB measured). The macOS
    /// watchdog must stop each surface at the 2 GiB budget within seconds.
    /// Manual only: if the watchdog regressed, this page would fill a CI
    /// runner's memory. Run under `scripts/memory-guard.py` with
    /// `cargo test --test worker_pdfium_integration heavy_page_memory_runaway -- --ignored`.
    #[tokio::test]
    #[ignore = "manual: unbounded memory if the watchdog regresses"]
    async fn heavy_page_memory_runaway_is_stopped_by_the_watchdog() {
        let path = fixture("heavy-page.pdf");
        let broker = broker();
        for (generation, (command, timeout)) in [
            ("render-page", CommandTimeout::RenderPage),
            ("render-thumbnail", CommandTimeout::RenderThumbnail),
            ("text-layout", CommandTimeout::TextLayout),
        ]
        .into_iter()
        .enumerate()
        {
            let key = WorkerDocumentKey {
                session_id: "heavy-page-memory".to_string(),
                content_generation: generation as u64 + 1,
            };
            let (elapsed, outcome) =
                timed_heavy_page_command(&broker, &key, &path, 1, command, timeout).await;
            eprintln!(
                "heavy-page page=1 command={command} outcome={outcome} elapsed_ms={:.0}",
                elapsed.as_secs_f64() * 1000.0
            );
            assert_eq!(outcome, "PDFIUM_WORKER_MEMORY_LIMIT", "{command}");
            assert!(
                elapsed < std::time::Duration::from_secs(6),
                "{command}: {elapsed:?}"
            );
        }
        // The light page still renders on the replacement worker.
        let key = WorkerDocumentKey {
            session_id: "heavy-page-memory".to_string(),
            content_generation: 10,
        };
        let (_, outcome) = timed_heavy_page_command(
            &broker,
            &key,
            &path,
            0,
            "render-page",
            CommandTimeout::RenderPage,
        )
        .await;
        assert_eq!(outcome, "ok");
    }

    /// Baseline measurement, not a regression test: run with
    /// `cargo test --test worker_pdfium_integration heavy_page_command_timings -- --ignored --nocapture`.
    /// `HEAVY_PAGE_PDF` overrides the fixture path. Uses the production
    /// timeout classes, so a timed-out command really waits for its limit.
    #[tokio::test]
    #[ignore = "manual worker baseline measurement; takes up to a minute"]
    async fn heavy_page_command_timings() {
        let path = std::env::var("HEAVY_PAGE_PDF").unwrap_or_else(|_| fixture("heavy-page.pdf"));
        let broker = broker();
        for (generation, (page_index, command, timeout)) in [
            (0, "render-page", CommandTimeout::RenderPage),
            (1, "render-page", CommandTimeout::RenderPage),
            (1, "render-thumbnail", CommandTimeout::RenderThumbnail),
            (1, "text-layout", CommandTimeout::TextLayout),
            (2, "render-page", CommandTimeout::RenderPage),
            (2, "render-thumbnail", CommandTimeout::RenderThumbnail),
            (2, "text-layout", CommandTimeout::TextLayout),
            (3, "render-page", CommandTimeout::RenderPage),
        ]
        .into_iter()
        .enumerate()
        {
            // A separate generation per command keeps each one's failure
            // record (and so the page quarantine) out of the next measurement.
            let key = WorkerDocumentKey {
                session_id: "heavy-page-timings".to_string(),
                content_generation: generation as u64 + 1,
            };
            let (elapsed, outcome) =
                timed_heavy_page_command(&broker, &key, &path, page_index, command, timeout).await;
            eprintln!(
                "heavy-page page={page_index} command={command} outcome={outcome} elapsed_ms={:.0}",
                elapsed.as_secs_f64() * 1000.0
            );
        }
    }

    fn solid_color_fixture() -> (tempfile::TempDir, String) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("solid-rgb.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();
        for (red, green, blue) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
            let content = Content {
                operations: vec![
                    Operation::new("rg", vec![red.into(), green.into(), blue.into()]),
                    Operation::new("re", vec![0.into(), 0.into(), 200.into(), 200.into()]),
                    Operation::new("f", vec![]),
                ],
            };
            let content_id =
                document.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
            page_ids.push(document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
                "Contents" => content_id,
            }));
        }
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => i64::try_from(page_ids.len()).unwrap(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        (directory, path.to_string_lossy().to_string())
    }

    /// Golden channel-order contract: the worker's `PageRendered.data` is
    /// straight RGBA. Pure red, green and blue pages must decode to exactly
    /// those channels for both the page and thumbnail render commands.
    #[tokio::test]
    async fn rendered_pixels_are_rgba_for_pure_red_green_blue_pages() {
        let (_directory, path) = solid_color_fixture();
        let broker = broker();
        let key = key("worker-rgba-golden");
        let admission = broker.admit_read(&key, None).await.unwrap();
        broker
            .ensure_document(&admission, key.clone(), path, None, None)
            .await
            .unwrap();
        let expected: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        for (page_index, expected) in expected.into_iter().enumerate() {
            let page_index = page_index as u32;
            for thumbnail in [false, true] {
                let rendered = broker
                    .command_for_document(
                        &admission,
                        &key,
                        |document_id| {
                            if thumbnail {
                                WorkerCommand::RenderThumbnail {
                                    document_id,
                                    page_index,
                                    target_width: 64,
                                }
                            } else {
                                WorkerCommand::RenderPage {
                                    document_id,
                                    page_index,
                                    target: WorkerRenderTarget::Scale(1.0),
                                    rotation: 0,
                                }
                            }
                        },
                        if thumbnail {
                            CommandTimeout::RenderThumbnail
                        } else {
                            CommandTimeout::RenderPage
                        },
                        None,
                    )
                    .await
                    .unwrap();
                let WorkerResult::PageRendered {
                    width,
                    height,
                    data,
                } = rendered
                else {
                    panic!("expected rendered page");
                };
                assert_eq!(data.len(), width as usize * height as usize * 4);
                let (pixels, _) = data.as_chunks::<4>();
                assert!(
                    pixels.iter().all(|pixel| *pixel == expected),
                    "page {page_index} thumbnail={thumbnail}: first pixel {:?}, expected {expected:?}",
                    &data[..4]
                );
            }
        }
        drop(admission);
        broker.shutdown().await.unwrap();
    }

    /// 50 A4 text pages; the same per-page workload as the worker-internal
    /// `render_page_a4_timing` baseline in `src/worker/pdfium.rs`.
    fn a4_text_fixture(page_count: usize) -> (tempfile::TempDir, String) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a4-scroll-baseline.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let mut page_ids = Vec::new();
        for page in 0..page_count {
            let mut operations = vec![
                Operation::new("rg", vec![0.85.into(), 0.2.into(), 0.2.into()]),
                Operation::new("re", vec![40.into(), 760.into(), 515.into(), 40.into()]),
                Operation::new("f", vec![]),
                Operation::new("RG", vec![0.1.into(), 0.3.into(), 0.8.into()]),
                Operation::new("w", vec![2.into()]),
                Operation::new("m", vec![40.into(), 60.into()]),
                Operation::new(
                    "c",
                    vec![
                        200.into(),
                        160.into(),
                        400.into(),
                        0.into(),
                        555.into(),
                        90.into(),
                    ],
                ),
                Operation::new("S", vec![]),
                Operation::new("rg", vec![0.into(), 0.into(), 0.into()]),
            ];
            for line in 0..60i64 {
                operations.extend([
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 10.into()]),
                    Operation::new("Td", vec![40.into(), (740 - line * 11).into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::String(
                            format!("{page:02}.{line:02} The quick brown fox jumps over the lazy dog; render baseline text line.")
                                .into_bytes(),
                            StringFormat::Literal,
                        )],
                    ),
                    Operation::new("ET", vec![]),
                ]);
            }
            let content_id = document.add_object(Stream::new(
                dictionary! {},
                Content { operations }.encode().unwrap(),
            ));
            page_ids.push(document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
                "Contents" => content_id,
            }));
        }
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => i64::try_from(page_ids.len()).unwrap(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        (directory, path.to_string_lossy().to_string())
    }

    /// Manual render-pipeline baseline through the real worker subprocess:
    /// single A4 page round trips and a 50-page sequential scroll proxy at
    /// scale 2.0. `RENDER_BASELINE_WORKER_BIN` may point at a shipping
    /// `cargo build --release` binary. Run from `src-tauri`:
    /// `TAURI_RESOURCE_DIR=$PWD cargo test --release --test worker_pdfium_integration -- --ignored --nocapture render_pipeline_scroll_baseline`
    #[tokio::test]
    #[ignore = "manual render baseline measurement"]
    async fn render_pipeline_scroll_baseline() {
        const PAGES: u32 = 50;
        let (_directory, path) = a4_text_fixture(PAGES as usize);
        let executable = std::env::var_os("RENDER_BASELINE_WORKER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")));
        eprintln!("render-baseline worker binary: {}", executable.display());
        let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
            executable,
            vec![OsString::from("--run-worker")],
        )));
        let key = key("render-scroll-baseline");
        let admission = broker.admit_read(&key, None).await.unwrap();
        broker
            .ensure_document(&admission, key.clone(), path, None, None)
            .await
            .unwrap();
        let render = |page_index: u32| {
            let broker = &broker;
            let admission = &admission;
            let key = &key;
            async move {
                let started = std::time::Instant::now();
                let rendered = broker
                    .command_for_document(
                        admission,
                        key,
                        |document_id| WorkerCommand::RenderPage {
                            document_id,
                            page_index,
                            target: WorkerRenderTarget::Scale(2.0),
                            rotation: 0,
                        },
                        CommandTimeout::RenderPage,
                        None,
                    )
                    .await
                    .unwrap();
                let WorkerResult::PageRendered { data, .. } = rendered else {
                    panic!("expected rendered page");
                };
                std::hint::black_box(data);
                started.elapsed().as_secs_f64() * 1000.0
            }
        };

        for _ in 0..5 {
            render(0).await;
        }
        let mut single = Vec::new();
        for _ in 0..40 {
            single.push(render(0).await);
        }
        single.sort_by(f64::total_cmp);
        eprintln!(
            "render-baseline single_page_round_trip: n={} min_ms={:.2} median_ms={:.2} p95_ms={:.2}",
            single.len(),
            single[0],
            single[single.len() / 2],
            single[(single.len() - 1) * 95 / 100]
        );

        let mut scrolls = Vec::new();
        for _ in 0..7 {
            let started = std::time::Instant::now();
            for page_index in 0..PAGES {
                render(page_index).await;
            }
            scrolls.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        eprintln!("render-baseline scroll_50_pages_runs_ms: {scrolls:.1?}");
        scrolls.sort_by(f64::total_cmp);
        eprintln!(
            "render-baseline scroll_50_pages: n={} min_ms={:.1} median_ms={:.1} max_ms={:.1}",
            scrolls.len(),
            scrolls[0],
            scrolls[scrolls.len() / 2],
            scrolls[scrolls.len() - 1]
        );

        drop(admission);
        broker.shutdown().await.unwrap();
    }

    fn form_fixture() -> (tempfile::TempDir, String) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker-form.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let text_id = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => 4, "P" => page_id,
            "T" => Object::String(b"name".to_vec(), StringFormat::Literal),
            "V" => Object::String(b"Ada".to_vec(), StringFormat::Literal),
            "DA" => Object::String(b"/Helv 10 Tf 0 g".to_vec(), StringFormat::Literal),
            "Rect" => vec![50.into(), 700.into(), 300.into(), 720.into()],
        });
        let on = document.add_object(Stream::new(dictionary! {}, b"0 g 2 2 12 12 re f".to_vec()));
        let off = document.add_object(Stream::new(dictionary! {}, Vec::new()));
        let checkbox_id = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "F" => 4, "P" => page_id,
            "T" => Object::String(b"agree".to_vec(), StringFormat::Literal),
            "V" => "On", "AS" => "On",
            "AP" => dictionary! { "N" => dictionary! { "On" => on, "Off" => off } },
            "Rect" => vec![50.into(), 650.into(), 66.into(), 666.into()],
        });
        // Reachable only through the page's /Annots, not the form's /Fields.
        let orphan_id = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => 4, "P" => page_id,
            "T" => Object::String(b"orphan".to_vec(), StringFormat::Literal),
            "V" => Object::String(b"adopted".to_vec(), StringFormat::Literal),
            "DA" => Object::String(b"/Helv 10 Tf 0 g".to_vec(), StringFormat::Literal),
            "Rect" => vec![50.into(), 600.into(), 300.into(), 620.into()],
        });
        let content_id = document.add_object(Stream::new(dictionary! {}, Vec::new()));
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Annots" => vec![
                    Object::Reference(text_id),
                    Object::Reference(checkbox_id),
                    Object::Reference(orphan_id),
                ],
            }),
        );
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
            }),
        );
        let acroform_id = document.add_object(dictionary! {
            "Fields" => vec![Object::Reference(text_id), Object::Reference(checkbox_id)],
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => font_id } },
        });
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id, "AcroForm" => acroform_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        (directory, path.to_string_lossy().to_string())
    }

    #[tokio::test]
    async fn worker_reads_form_info_and_page_form_fields() {
        use preview_alternative_lib::forms::{FormFieldKind, FormType};

        let broker = broker();
        let (_directory, path) = form_fixture();
        for (session, source, expect_form) in [
            ("worker-form", path, true),
            ("worker-no-form", fixture("reorder-rich.pdf"), false),
        ] {
            let key = key(session);
            let admission = broker.admit_read(&key, None).await.unwrap();
            if let Err(error) = broker
                .ensure_document(&admission, key.clone(), source, None, None)
                .await
            {
                panic!(
                    "worker open failed: {error:?}; stderr={}",
                    broker.last_worker_stderr_for_test().await
                );
            }

            let info = broker
                .command_for_document(
                    &admission,
                    &key,
                    |document_id| WorkerCommand::GetFormInfo { document_id },
                    CommandTimeout::DocumentRead,
                    None,
                )
                .await
                .unwrap();
            let WorkerResult::FormInfo(info) = info else {
                panic!("unexpected form info result: {info:?}");
            };
            assert_eq!(info.signature_count, 0);

            let fields = broker
                .command_for_document(
                    &admission,
                    &key,
                    |document_id| WorkerCommand::GetPageFormFields {
                        document_id,
                        page_index: 0,
                    },
                    CommandTimeout::DocumentRead,
                    None,
                )
                .await
                .unwrap();
            let WorkerResult::PageFormFields(fields) = fields else {
                panic!("unexpected form fields result: {fields:?}");
            };

            if !expect_form {
                assert_eq!(info.form_type, FormType::None);
                assert!(fields.is_empty());
                continue;
            }
            assert_eq!(info.form_type, FormType::AcroForm);
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0].field_name, "name");
            assert_eq!(fields[0].kind, FormFieldKind::Text);
            assert_eq!(fields[0].value.as_deref(), Some("Ada"));
            assert_eq!(fields[0].annot_index, 0);
            let rect = fields[0].normalized_rect.clone().unwrap();
            assert!((rect.x - 50.0 / 612.0).abs() < 1e-3, "{rect:?}");
            assert!((rect.y - 72.0 / 792.0).abs() < 1e-3, "{rect:?}");
            assert_eq!(fields[1].field_name, "agree");
            assert_eq!(fields[1].kind, FormFieldKind::Checkbox);
            assert_eq!(fields[1].checked, Some(true));
            assert_eq!(fields[2].field_name, "orphan");
            assert_eq!(fields[2].value.as_deref(), Some("adopted"));
            assert_eq!(fields[2].annot_index, 2);
        }
    }
}
