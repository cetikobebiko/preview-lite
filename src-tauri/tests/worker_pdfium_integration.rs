#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Document, Object, Stream, StringFormat};
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        CommandTimeout, PdfiumWorkerBroker, WorkerDocumentKey, WorkerLauncher,
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
                |document_id| WorkerCommand::ValidatePdfUa { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::PdfUaValidation(report)
                if !report.is_tagged && !report.has_document_title && report.score == 50
        ));

        let structure_error = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetStructureTree { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            structure_error.code,
            "PDFIUM_WORKER_STRUCTURE_TREE_UNAVAILABLE"
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
                |document_id| WorkerCommand::ValidatePdfUa { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::PdfUaValidation(report) if !report.is_tagged
        ));
        drop(admission);
        let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
        broker.invalidate_document(&exclusive, &key).await.unwrap();
        drop(exclusive);
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
                |document_id| WorkerCommand::ValidatePdfUa { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            pdf_ua,
            WorkerResult::PdfUaValidation(report) if !report.is_tagged
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
}
