import { existsSync, readFileSync, readdirSync } from "node:fs";
import { extname, join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CONTRACTS,
  PROTECTED_CONTRACT_IDS,
} from "./contracts.js";
import {
  REDACTION_ERROR_CODES,
  REDACTION_LIMITS,
  REDACTION_SCHEMA_VERSION,
  U32_MIN,
  U32_MAX,
  I32_MIN,
  I32_MAX,
} from "../../features/redaction/redactionSchemas.js";

const sourceRoot = resolve(process.cwd(), "src");
const clientPath = resolve(sourceRoot, "services/backend/client.js");
const rustLibPath = resolve(process.cwd(), "src-tauri/src/lib.rs");
const rustSourceRoot = resolve(process.cwd(), "src-tauri/src");
const documentSessionPath = resolve(rustSourceRoot, "document_session.rs");

// Windows checkout'ları CRLF ve `\` ayırıcı üretir; sözleşmeler LF metin ve
// POSIX yollarla yazıldığı için kaynak her platformda aynı biçime getirilir.
function readSource(path) {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}

function relativePosix(from, to) {
  return relative(from, to).split(sep).join("/");
}

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [path];
  }).filter((path) => [".js", ".ts", ".svelte"].includes(extname(path)));
}

function rustSourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return rustSourceFiles(path);
    return extname(path) === ".rs" ? [path] : [];
  });
}

function rustFunctionBody(source, functionName) {
  const signature = new RegExp(
    `\\b(?:pub(?:\\(crate\\))?\\s+)?(?:async\\s+)?fn\\s+${functionName}\\b`,
  );
  const match = signature.exec(source);
  if (!match) throw new Error(`Rust function not found: ${functionName}`);
  const opening = source.indexOf("{", match.index);
  if (opening < 0) throw new Error(`Rust function body not found: ${functionName}`);
  let depth = 0;
  for (let index = opening; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(opening, index + 1);
    }
  }
  throw new Error(`Rust function body is unbalanced: ${functionName}`);
}

describe("backend service boundaries", () => {
  it("allows the Tauri invoke import only in client.js", () => {
    const coreImport =
      /import\s+(?:(?!\bfrom\b)[\s\S])*?\bfrom\s*["']@tauri-apps\/api\/core["']|import\s*["']@tauri-apps\/api\/core["']/;
    const offenders = sourceFiles(sourceRoot)
      .filter((path) => path !== clientPath)
      .filter((path) => coreImport.test(readSource(path)))
      .map((path) => relativePosix(sourceRoot, path));

    expect(offenders).toEqual([]);
  });

  it("does not expose protected raw wrappers from the public barrel", () => {
    const barrel = readSource(
      resolve(sourceRoot, "services/backend/index.js"),
    );
    const protectedNames = PROTECTED_CONTRACT_IDS.filter((name) =>
      Object.prototype.hasOwnProperty.call(CONTRACTS, name),
    );

    for (const name of protectedNames) {
      expect(barrel).not.toMatch(new RegExp(`\\b${name}\\b`));
    }
  });

  it("prevents components from importing protected raw wrappers", () => {
    const protectedNames = PROTECTED_CONTRACT_IDS.join("|");
    const protectedImport = new RegExp(
      `import\\s*\\{[^}]*\\b(?:${protectedNames})\\b[^}]*\\}\\s*from\\s*["'][^"']*services/backend/`,
      "s",
    );
    const offenders = sourceFiles(resolve(sourceRoot, "components"))
      .filter((path) => path.endsWith(".svelte"))
      .filter((path) => protectedImport.test(readSource(path)))
      .map((path) => relativePosix(sourceRoot, path));

    expect(offenders).toEqual([]);
  });

  it("ensures old render commands are absent from executable public handlers and session render commands are used", () => {
    const handlerRegistry = readSource(rustLibPath);

    // Old render commands must NOT be registered in Tauri generate_handler!
    expect(handlerRegistry).not.toMatch(/^\s*render_page\s*,/m);
    expect(handlerRegistry).not.toMatch(/^\s*render_thumbnail\s*,/m);
    expect(handlerRegistry).not.toMatch(/^\s*set_active_render_document\s*,/m);
    expect(handlerRegistry).not.toMatch(/^\s*cancel_render_request\s*,/m);

    // Session render commands must be present in handler registry
    expect(handlerRegistry).toMatch(/document_session::render_session_page/);
    expect(handlerRegistry).toMatch(/document_session::render_session_thumbnail/);
    expect(handlerRegistry).toMatch(/document_session::set_active_session_render_document/);
    expect(handlerRegistry).toMatch(/document_session::cancel_session_render_request/);
  });

  it("keeps every interactive session PDF read command on the worker broker boundary", () => {
    const source = readSource(documentSessionPath);
    const functions = [
      "open_document_session",
      "authenticate_document_session",
      "probe_candidate_render",
      "get_session_page_count",
      "get_session_page_sizes",
      "get_session_pdf_outline",
      "get_session_pdf_metadata",
      "get_session_structure_tree",
      "audit_session_accessibility",
      "get_session_page_annotations",
      "get_session_form_info",
      "get_session_page_form_fields",
      "render_session_page",
      "render_session_thumbnail",
      "get_session_page_text_layout",
      "search_session_pdf_text",
    ];
    const forbidden = [
      "with_pdfium(",
      "get_pdfium(",
      "load_pdf_from_file(",
      "render_page_rgba(",
      "search_pdf_text(",
      "get_page_text_layout(",
    ];

    for (const functionName of functions) {
      const body = rustFunctionBody(source, functionName);
      for (const call of forbidden) {
        expect(body, `${functionName} must not call ${call}`).not.toContain(call);
      }
      expect(body, `${functionName} must use the worker/session boundary`).toMatch(
        /worker_broker|acquire_worker_read|render_viewer_request_with_renderer/,
      );
    }
  });

  it("does not retain raw-path interactive PDF read handlers or helpers", () => {
    const handlerRegistry = readSource(rustLibPath);
    const rustSource = rustSourceFiles(rustSourceRoot)
      .map((path) => readSource(path))
      .join("\n");
    const legacyFunctions = [
      "check_pdf_encrypted",
      "authenticate_pdf_password",
      "get_page_count",
      "get_pdf_outline",
      "get_page_text_layout",
      "search_pdf_text",
      "get_page_annotations",
      "get_structure_tree",
      "validate_pdf_ua",
    ];

    for (const functionName of legacyFunctions) {
      expect(handlerRegistry).not.toMatch(
        new RegExp(`^\\s*(?:[a-z_]+::)*${functionName}\\s*,`, "m"),
      );
      expect(rustSource).not.toMatch(
        new RegExp(`\\b(?:pub\\s+)?(?:async\\s+)?fn\\s+${functionName}\\b`),
      );
    }
  });

  it("rejects unauthorized filePath arguments for session PDF operations in frontend contracts", () => {
    const sessionContractKeys = [
      "undoSessionMutation",
      "redoSessionMutation",
      "rotateSessionPage",
      "deleteSessionPages",
      "reorderSessionPages",
      "insertSessionBlankPage",
      "addSessionAnnotation",
      "deleteSessionAnnotation",
      "fillSessionFormFields",
    ];

    for (const key of sessionContractKeys) {
      const contract = CONTRACTS[key];
      expect(contract).toBeDefined();
      if (contract && contract.payload && contract.payload.required) {
        const requiredKeys = Object.keys(contract.payload.required);
        expect(requiredKeys).toContain("sessionId");
        expect(requiredKeys).not.toContain("filePath");
        expect(requiredKeys).not.toContain("displayPath");
      }
    }
  });

  it("keeps legacy active-PDF path contracts out of the public frontend surface", () => {
    const forbidden = [
      "renderPage",
      "renderThumbnail",
      "setActiveRenderDocument",
      "cancelRenderRequest",
      "getPageCount",
      "getPdfOutline",
      "searchPdfText",
      "getPageTextLayout",
      "getPdfMetadata",
      "setPdfMetadata",
      "getStructureTree",
      "updateAltText",
      "validatePdfUa",
      "rotatePage",
      "deletePages",
      "insertBlankPage",
      "reorderPages",
      "getPageAnnotations",
      "addAnnotation",
      "deleteAnnotation",
      "savePdfWithAnnotations",
      "analyzeColorSpaces",
      "calculateInkCoverage",
      "renderSeparationChannels",
      "exportPdfX",
    ];
    for (const key of forbidden) {
      expect(CONTRACTS).not.toHaveProperty(key);
    }
  });

  it("removes unregistered legacy frontend command surfaces", () => {
    for (const key of [
      "checkPdfEncrypted",
      "authenticatePdfPassword",
      "createSessionJobSnapshot",
      "askLocalAi",
    ]) {
      expect(CONTRACTS).not.toHaveProperty(key);
    }

    const frontendSource = sourceFiles(sourceRoot)
      .filter((path) => !path.endsWith(".test.js"))
      .map((path) => readSource(path))
      .join("\n");
    for (const command of [
      "check_pdf_encrypted",
      "authenticate_pdf_password",
      "create_session_job_snapshot",
      "ask_local_ai",
    ]) {
      expect(frontendSource).not.toContain(command);
    }
  });

  it("uses session identity for PDF OCR, signature verification and PDF share while isolating external share", () => {
    for (const key of [
      "detectScannedPages",
      "runOcrOnPage",
      "performOcrOnPage",
      "shareFileMacos",
    ]) {
      const required = Object.keys(CONTRACTS[key].payload.required);
      expect(required).toContain("sessionId");
      expect(required).toContain("expectedContentGeneration");
      expect(required).not.toContain("path");
      expect(required).not.toContain("filePath");
    }
    expect(Object.keys(CONTRACTS.shareExternalFileMacos.payload.required))
      .toEqual(["path"]);
  });

  it("does not expose working-copy paths or path-based render identity in frontend source", () => {
    const contractSource = readSource(
      resolve(sourceRoot, "services/backend/contracts.js"),
    );
    const renderSessionSource = readSource(
      resolve(sourceRoot, "services/renderSession.js"),
    );
    const annotationLayerSource = readSource(
      resolve(sourceRoot, "components/AnnotationLayer.svelte"),
    );

    expect(contractSource).not.toMatch(/\bworkingPath\b|\bworking_path\b/);
    expect(renderSessionSource).not.toMatch(/\bpath\b|\bdocumentId\b|\bgeneration\b/);
    expect(annotationLayerSource).not.toMatch(/\bfilePath\b/);
  });

  it("keeps prepress and search active identity pathless", () => {
    const prepressSource = readSource(
      resolve(sourceRoot, "components/PrepressPanel.svelte"),
    );
    const searchSource = readSource(
      resolve(sourceRoot, "utils/searchCoordinator.js"),
    );
    const prepressStateSource = readSource(
      resolve(sourceRoot, "stores/prepressState.svelte.js"),
    );
    expect(prepressSource).not.toMatch(/\bfilePath\b/);
    expect(searchSource).not.toMatch(/\bfilePath\b|\bdocumentGenerationId\b/);
    expect(prepressStateSource).toMatch(/createPrepressContextKey/);
    expect(prepressStateSource).toMatch(/isCurrentPrepressEvent/);
    expect(prepressStateSource).not.toMatch(/\bactivePdfDisplayPath\b|\bfilePath\b/);
  });

  it("registers existing session-native OCR names and separate external-file share", () => {
    const handlerRegistry = readSource(rustLibPath);
    expect(handlerRegistry).toMatch(/ocr::detect_scanned_pages\b/);
    expect(handlerRegistry).not.toMatch(/ocr::detect_scanned_page\b/);
    expect(handlerRegistry).toMatch(/ocr::run_ocr_on_page/);
    expect(handlerRegistry).toMatch(/\bperform_ocr_on_page\b/);
    expect(handlerRegistry).toMatch(/\bshare_file_macos\b/);
    expect(handlerRegistry).toMatch(/\bshare_external_file_macos\b/);
    expect(handlerRegistry).not.toMatch(/\bperform_session_ocr_on_page\b/);
    expect(handlerRegistry).not.toMatch(/\bshare_session_macos\b/);
  });

  it("keeps synchronous Tauri commands to an audited, non-blocking allowlist", () => {
    // Senkron Tauri komutları ana (UI) thread'de çalışır. Yeni komutlar async olmalı;
    // senkron kalanlar yalnız hızlı, bloklamayan işler yapar ve burada gerekçelendirilir.
    const syncCommandAllowlist = {
      // Anında FEATURE_UNAVAILABLE döndürür.
      trigger_continuity_camera_scan: "lib.rs",
      // Yalnız bellek içi mutex/LRU işlemleri; generate_handler! içinde kayıtlı değiller.
      set_active_render_document: "pdf_engine/mod.rs",
      cancel_render_request: "pdf_engine/mod.rs",
      clear_search_cache: "pdf_engine/mod.rs",
    };
    const blockingCall =
      /std::thread::sleep|std::fs::|std::process::Command|\.try_wait\(|\.wait\(|\.join\(\)|run_child_with_timeout|run_with_timeout|reqwest::blocking|block_on\(/;
    const commandDefinition =
      /#\[tauri::command\]\s*(?:#\[[^\]]*\]\s*|\/\/[^\n]*\n\s*)*(?:pub(?:\([^)]*\))?\s+)?(async\s+)?fn\s+(\w+)/g;

    let attributeCount = 0;
    const commands = rustSourceFiles(rustSourceRoot).flatMap((path) => {
      const source = readSource(path);
      attributeCount += (source.match(/#\[tauri::command\]/g) ?? []).length;
      return [...source.matchAll(commandDefinition)].map((match) => {
        const opening = source.indexOf("{", match.index + match[0].length);
        let depth = 0;
        let end = opening;
        for (; end < source.length; end += 1) {
          if (source[end] === "{") depth += 1;
          if (source[end] === "}" && --depth === 0) break;
        }
        return {
          file: relativePosix(rustSourceRoot, path),
          name: match[2],
          isAsync: Boolean(match[1]),
          body: source.slice(opening, end + 1),
        };
      });
    });

    // Ayrıştırıcı her komut özniteliğini bir fonksiyona bağlayabilmeli.
    expect(commands.length).toBe(attributeCount);
    expect(commands.length).toBeGreaterThan(0);

    const syncCommands = commands.filter((command) => !command.isAsync);
    expect(
      Object.fromEntries(syncCommands.map((command) => [command.name, command.file])),
    ).toEqual(syncCommandAllowlist);

    const blockingSyncCommands = syncCommands
      .filter((command) => blockingCall.test(command.body))
      .map((command) => `${command.file}:${command.name}`);
    expect(blockingSyncCommands).toEqual([]);

    const externalShare = commands.filter(
      (command) => command.name === "share_external_file_macos",
    );
    expect(externalShare).toHaveLength(2);
    expect(externalShare.every((command) => command.isAsync)).toBe(true);
    expect(externalShare.some((command) => /spawn_blocking/.test(command.body))).toBe(true);
  });

  it("keeps unsupported metadata writes session-native and removes accessibility write bypasses", () => {
    const handlerRegistry = readSource(rustLibPath);
    const sessionSource = readSource(
      resolve(rustSourceRoot, "document_session.rs"),
    );
    expect(handlerRegistry).toMatch(/document_session::update_session_metadata/);
    expect(handlerRegistry).not.toMatch(/document_session::update_session_alt_text/);
    expect(handlerRegistry).not.toMatch(/^\s*set_pdf_metadata\s*,/m);
    expect(handlerRegistry).not.toMatch(/^\s*update_alt_text\s*,/m);
    expect(sessionSource).not.toMatch(/fn update_session_alt_text/);
    for (const handler of ["update_session_metadata"]) {
      const start = sessionSource.indexOf(`fn ${handler}`);
      const body = sessionSource.slice(start, start + 900);
      expect(start).toBeGreaterThanOrEqual(0);
      expect(body).toMatch(/reject_unsupported_session_write/);
      expect(body).not.toMatch(/\.mutate\(|dirty\s*=|bump_revision/);
    }
    const helperStart = sessionSource.indexOf(
      "fn reject_unsupported_session_write",
    );
    const helperBody = sessionSource.slice(helperStart, helperStart + 900);
    expect(helperStart).toBeGreaterThanOrEqual(0);
    expect(helperBody).toMatch(/UNSUPPORTED_PDFIUM_CAPABILITY/);
    expect(helperBody).not.toMatch(/\.mutate\(|dirty\s*=|bump_revision/);
  });

  it("does not re-export legacy active-PDF path facades from pdfState or the backend barrel", () => {
    const pdfStateSource = readSource(
      resolve(sourceRoot, "pdfState.js"),
    );
    const barrelSource = readSource(
      resolve(sourceRoot, "services/backend/index.js"),
    );
    const forbiddenNames = [
      "getPdfMetadata",
      "setPdfMetadata",
      "savePdfWithAnnotations",
      "reorderPdfPages",
      "printPdf",
      "exportPagesAsPdfX",
      "getFormFields",
      "updateFormField",
    ];

    for (const name of forbiddenNames) {
      expect(pdfStateSource).not.toMatch(
        new RegExp(`export\\s+(?:async\\s+)?function\\s+${name}\\b|export\\s+const\\s+${name}\\b`),
      );
      expect(barrelSource).not.toMatch(new RegExp(`\\b${name}\\b`));
    }
  });

  it("ensures no signature UI components remain in the component tree", () => {
    for (const file of [
      "components/SignaturePanel.svelte",
      "components/SignatureWizardModal.svelte",
      "components/SignatureSettingsModal.svelte",
    ]) {
      expect(existsSync(resolve(sourceRoot, file))).toBe(false);
    }
  });

  it("isolates every active snapshot PDF job from the Tauri process", () => {
    const rustLib = readSource(rustLibPath);
    const mainSource = readSource(resolve(rustSourceRoot, "main.rs"));
    const jobWorkerSource = readSource(
      resolve(rustSourceRoot, "job_worker.rs"),
    );
    const sources = {
      documentSession: readSource(documentSessionPath),
      ocr: readSource(resolve(rustSourceRoot, "ocr.rs")),
      ai: readSource(resolve(rustSourceRoot, "ai.rs")),
      redaction: readSource(resolve(rustSourceRoot, "redaction.rs")),
    };
    const forbidden = [
      "with_pdfium(",
      "get_pdfium(",
      "load_pdf_from_file(",
      "open_pdfium_doc(",
      "render_page_rgba(",
      "Document::load(",
    ];
    const isolated = [
      [sources.documentSession, "recover_document_session"],
      [sources.documentSession, "export_session_page_to_image"],
      [sources.documentSession, "export_session_pages_as_images"],
      [sources.documentSession, "export_session_text_content"],
      [sources.documentSession, "export_session_text_to_file"],
      [sources.documentSession, "print_session_pdf"],
      [sources.documentSession, "inspect_session_print_resources"],
      [sources.documentSession, "generate_session_rgb_derived_coverage_preview"],
      [sources.documentSession, "generate_session_rgb_derived_process_preview"],
      [sources.documentSession, "split_session_pdf"],
      [sources.documentSession, "merge_session_pdfs"],
      [sources.documentSession, "compare_session_pdf"],
      [sources.documentSession, "extract_session_pages"],
      [sources.ocr, "detect_scanned_pages_for_session"],
      [sources.ocr, "run_ocr_on_page_for_session"],
      [sources.ai, "extract_semantic_page_texts"],
      [sources.redaction, "scan_sensitive_data"],
      [sources.redaction, "plan_secure_redaction"],
      [rustLib, "perform_ocr_on_page"],
    ];

    for (const [source, functionName] of isolated) {
      const body = rustFunctionBody(source, functionName);
      for (const call of forbidden) {
        expect(body, `${functionName} must not call ${call}`).not.toContain(call);
      }
      expect(body, `${functionName} must dispatch an ephemeral job worker`).toMatch(
        /job_worker::(?:run_job|JobCommand)/,
      );
    }

    const snapshotRunner = rustFunctionBody(
      sources.documentSession,
      "run_snapshot_job",
    );
    for (const call of forbidden) {
      expect(snapshotRunner, `run_snapshot_job must not call ${call}`).not.toContain(
        call,
      );
    }
    expect(snapshotRunner).toMatch(/JobCommand::ValidateSnapshot/);
    expect(snapshotRunner).toMatch(/session_job_context/);
    expect(mainSource).toMatch(/--run-job-worker/);
    expect(mainSource).toMatch(/--run-worker/);
    expect(jobWorkerSource).toMatch(/std::env::current_exe/);
    expect(jobWorkerSource).toMatch(/child_processes\.run_with_timeout/);
    expect(jobWorkerSource).toMatch(/validate_output_file/);
    expect(jobWorkerSource).toMatch(/MAX_JOB_REQUEST_BYTES:\s*u64\s*=\s*1024 \* 1024/);
    expect(jobWorkerSource).toMatch(/MAX_JOB_RESPONSE_BYTES:\s*u64\s*=\s*256 \* 1024 \* 1024/);

    const shareStart = rustLib.indexOf("fn share_file_macos");
    expect(shareStart).toBeGreaterThanOrEqual(0);
    expect(rustLib.slice(shareStart, shareStart + 900)).toMatch(
      /share_session_snapshot_with/,
    );
    const shareHelperStart = rustLib.indexOf("fn share_session_snapshot_with");
    expect(shareHelperStart).toBeGreaterThanOrEqual(0);
    expect(rustLib.slice(shareHelperStart, shareHelperStart + 900)).toMatch(
      /run_snapshot_job/,
    );

  });

  it("keeps session mutations and production atomic validation behind ephemeral workers", () => {
    const sessionSource = readSource(documentSessionPath);
    const documentIoSource = readSource(
      resolve(rustSourceRoot, "document_io.rs"),
    );
    const forbidden = [
      "with_pdfium(",
      "get_pdfium(",
      "load_pdf_from_file(",
      "open_pdfium_doc(",
    ];
    const handlers = [
      "rotate_session_page",
      "delete_session_pages",
      "reorder_session_pages",
      "insert_session_blank_page",
      "add_session_annotation",
      "delete_session_annotation",
      "fill_session_form_fields",
      "author_session_form_fields",
    ];

    for (const functionName of handlers) {
      const body = rustFunctionBody(sessionSource, functionName);
      for (const call of forbidden) {
        expect(body, `${functionName} must not call ${call}`).not.toContain(call);
      }
      expect(body, `${functionName} must use isolated mutation orchestration`)
        .toContain("mutate_isolated");
      expect(body, `${functionName} must carry a typed mutation recipe`)
        .toContain("MutationRecipe::");
    }

    // Undo/redo carry no recipe: they restore a history entry through the same isolated
    // transaction (commit, cancellation and recovery), so the same PDFium ban applies.
    for (const functionName of ["undo_session_mutation", "redo_session_mutation"]) {
      const body = rustFunctionBody(sessionSource, functionName);
      for (const call of forbidden) {
        expect(body, `${functionName} must not call ${call}`).not.toContain(call);
      }
      expect(body, `${functionName} must use isolated mutation orchestration`)
        .toContain("mutate_isolated_restore");
      expect(body, `${functionName} must name its history direction`)
        .toContain("HistoryDirection::");
    }
    const restore = rustFunctionBody(sessionSource, "mutate_isolated_restore");
    expect(restore).toContain("run_mutation_transaction");
    for (const call of forbidden) {
      expect(restore, `mutate_isolated_restore must not call ${call}`).not.toContain(call);
    }

    const orchestration =
      rustFunctionBody(sessionSource, "mutate_isolated") +
      rustFunctionBody(sessionSource, "execute_mutation_transaction") +
      rustFunctionBody(sessionSource, "run_mutation_worker") +
      rustFunctionBody(sessionSource, "run_validation_worker");
    expect(orchestration).toMatch(/admit_exclusive/);
    expect(orchestration).toMatch(/write_owned/);
    expect(orchestration).toMatch(/invalidate_document/);
    expect(orchestration).toMatch(/JobCommand::MutatePdf/);
    expect(orchestration).toMatch(/JobCommand::ValidateMutationOutput/);
    expect(orchestration).toMatch(/atomic_commit_prevalidated_pdf/);
    for (const call of forbidden) {
      expect(orchestration, `mutate_isolated must not call ${call}`).not.toContain(call);
    }

    const commit = rustFunctionBody(
      documentIoSource,
      "atomic_commit_prevalidated_pdf",
    );
    expect(commit).toMatch(/atomic_write_with/);
    expect(commit).toMatch(/validated_digest/);
    for (const call of forbidden) {
      expect(commit, `atomic commit must not call ${call}`).not.toContain(call);
    }

    const saveCopy = rustFunctionBody(documentIoSource, "atomic_copy_pdf");
    expect(saveCopy).toMatch(/#\[cfg\(not\(test\)\)\]/);
    expect(saveCopy).toMatch(/validate_pdf_file_isolated/);
  });
  it("fails disabled snapshot features before snapshot creation or PDF parsing", () => {
    const redactionSource = readSource(resolve(rustSourceRoot, "redaction.rs"));

    // 1. Helper signatures take 0 parameters
    expect(redactionSource).toMatch(/\bfn\s+apply_redactions_unavailable\s*\(\s*\)/);
    expect(redactionSource).toMatch(/\bfn\s+sanitize_pdf_document_unavailable\s*\(\s*\)/);

    // 2. Command bodies match exactly { helper_name() } after whitespace normalization
    const normalizeWs = (str) => str.replace(/\s+/g, " ").trim();

    const applyRedactionsBody = rustFunctionBody(redactionSource, "apply_redactions");
    expect(normalizeWs(applyRedactionsBody)).toBe("{ apply_redactions_unavailable() }");

    const sanitizeBody = rustFunctionBody(redactionSource, "sanitize_pdf_document");
    expect(normalizeWs(sanitizeBody)).toBe("{ sanitize_pdf_document_unavailable() }");

    // 3. Helper bodies only construct typed FEATURE_UNAVAILABLE error and contain no external calls
    const applyHelperBody = rustFunctionBody(redactionSource, "apply_redactions_unavailable");
    expect(applyHelperBody).toContain("FEATURE_UNAVAILABLE");
    expect(applyHelperBody).not.toMatch(/std::fs|File::|OpenOptions::|job_worker::|run_snapshot_job|get_pdfium|with_pdfium|load_pdf_from_file|Document::load/);

    const sanitizeHelperBody = rustFunctionBody(redactionSource, "sanitize_pdf_document_unavailable");
    expect(sanitizeHelperBody).toContain("FEATURE_UNAVAILABLE");
    expect(sanitizeHelperBody).not.toMatch(/std::fs|File::|OpenOptions::|job_worker::|run_snapshot_job|get_pdfium|with_pdfium|load_pdf_from_file|Document::load/);

    // 4. scan_sensitive_data negative & positive assertions
    const scanBody = rustFunctionBody(redactionSource, "scan_sensitive_data");
    const scanForbidden = [
      "get_pdfium(",
      "with_pdfium(",
      "load_pdf_from_file(",
      "Document::load(",
      "scan_sensitive_file(",
    ];
    for (const forbidden of scanForbidden) {
      expect(scanBody, `scan_sensitive_data must not contain ${forbidden}`).not.toContain(forbidden);
    }
    expect(scanBody).toContain("run_snapshot_job");
    expect(scanBody).toContain("job_worker::run_job");
    expect(scanBody).toContain("JobCommand::ScanSensitiveData");

    const cases = [
      [redactionSource, "apply_redactions"],
      [redactionSource, "sanitize_pdf_document"],
      [redactionSource, "apply_redactions_unavailable"],
      [redactionSource, "sanitize_pdf_document_unavailable"],
      [readSource(resolve(rustSourceRoot, "ocr.rs")), "export_as_searchable_pdf"],
    ];
    for (const [source, functionName] of cases) {
      const body = rustFunctionBody(source, functionName);
      expect(body, `${functionName} must return an explicit feature error or delegate to helper`).toMatch(
        /FEATURE_UNAVAILABLE|apply_redactions_unavailable|sanitize_pdf_document_unavailable/,
      );
      expect(body, `${functionName} must not reserve a snapshot`).not.toContain(
        "run_snapshot_job",
      );
      expect(body, `${functionName} must not parse a PDF`).not.toMatch(
        /with_pdfium\(|get_pdfium\(|load_pdf_from_file\(|Document::load\(/,
      );
      expect(body, `${functionName} must not perform filesystem or worker operations`).not.toMatch(
        /std::fs|File::|OpenOptions::|job_worker::/,
      );
    }

    const handlerRegistry = readSource(rustLibPath);
    expect(handlerRegistry).not.toContain("export_session_pages_as_pdf_x");
    expect(readSource(documentSessionPath)).not.toContain("export_session_pages_as_pdf_x");
    expect(handlerRegistry).not.toMatch(/^\s*burn_annotations_to_pdf\s*,/m);
    expect(handlerRegistry).not.toMatch(/^\s*manipulation::merge_pdfs\s*,/m);
  });

  it("secure-redaction contract manifest matches Rust serialization and frontend runtime validators", () => {
    const rustSource = readSource(resolve(rustSourceRoot, "secure_redaction.rs"));
    const manifestPath = resolve(process.cwd(), "src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json");
    const manifest = JSON.parse(readSource(manifestPath));

    expect(REDACTION_SCHEMA_VERSION).toBe(manifest.schemaVersion);
    expect(rustSource).toContain(`pub const REDACTION_SCHEMA_VERSION: &str = "${manifest.schemaVersion}";`);

    // Verify all 15 error codes match manifest, JS, and Rust
    const manifestErrorCodes = manifest.errorCodes;
    expect(manifestErrorCodes.length).toBe(15);
    const jsErrorCodes = Object.values(REDACTION_ERROR_CODES);
    expect(jsErrorCodes).toEqual(manifestErrorCodes);
    for (const code of manifestErrorCodes) {
      expect(rustSource, `Rust secure_redaction.rs missing error code: ${code}`).toContain(code);
    }

    // Verify all limits match manifest, JS, and Rust
    for (const [limitKey, limitVal] of Object.entries(manifest.limits)) {
      expect(REDACTION_LIMITS[limitKey], `JS REDACTION_LIMITS.${limitKey} mismatch`).toBe(limitVal);
      // Check Rust constant presence
      expect(rustSource, `Rust constant for ${limitKey} missing or mismatch`).toMatch(
        new RegExp(`pub const ${limitKey}:\\s*(?:usize|u32|u64|f64)\\s*=\\s*${limitVal}(?:\\.0)?;`),
      );
    }

    // Verify numeric ranges
    expect(U32_MIN).toBe(manifest.numericRanges.u32Min);
    expect(U32_MAX).toBe(manifest.numericRanges.u32Max);
    expect(I32_MIN).toBe(manifest.numericRanges.i32Min);
    expect(I32_MAX).toBe(manifest.numericRanges.i32Max);

    // Verify rotations and status enums in manifest match JS/Rust expectations
    expect(manifest.rotations).toEqual([0, 90, 180, 270]);
    expect(manifest.verificationStatuses).toEqual(["passed", "failed", "not_run"]);
  });

  it("enforces plan_secure_redaction boundary and isolation rules", () => {
    const redactionSource = readSource(resolve(rustSourceRoot, "redaction.rs"));
    const planBody = rustFunctionBody(redactionSource, "plan_secure_redaction");
    const forbidden = [
      "with_pdfium(",
      "get_pdfium(",
      "load_pdf_from_file(",
      "open_pdfium_doc(",
      "Document::load(",
    ];
    for (const call of forbidden) {
      expect(planBody, `plan_secure_redaction must not contain ${call}`).not.toContain(call);
    }
    expect(planBody).toContain("run_snapshot_job");
    expect(planBody).toContain("job_worker::run_job");
    expect(planBody).toContain("JobCommand::PlanSecureRedaction");

    const planContract = CONTRACTS.planSecureRedaction;
    expect(planContract).toBeDefined();
    expect(planContract.command).toBe("plan_secure_redaction");
    const payloadKeys = Object.keys(planContract.payload.required);
    expect(payloadKeys).toEqual([
      "jobId",
      "sessionId",
      "expectedContentGeneration",
      "policy",
      "acknowledgements",
      "regions",
    ]);

    const planCallsites = sourceFiles(sourceRoot)
      .filter((path) => readSource(path).includes("startSecureRedactionPlan"))
      .map((path) => relativePosix(sourceRoot, path))
      .filter((path) => !path.endsWith(".test.js") && path !== "services/backend/redaction.js");
    expect(planCallsites).toEqual(["services/app/secureRedactionController.js"]);
  });

  it("keeps secure execution worker-isolated and publication digest-bound", () => {
    const redactionSource = readSource(resolve(rustSourceRoot, "redaction.rs"));
    const workerSource = readSource(resolve(rustSourceRoot, "job_worker.rs"));
    const documentIoSource = readSource(resolve(rustSourceRoot, "document_io.rs"));
    const publicExecutionBody = rustFunctionBody(redactionSource, "execute_secure_redaction_core");
    const executionBody = rustFunctionBody(
      redactionSource,
      "execute_secure_redaction_core_internal",
    );

    expect(publicExecutionBody).toContain("execute_secure_redaction_core_internal");
    expect(publicExecutionBody).toContain("None");
    expect(executionBody).toContain("run_snapshot_job");
    expect(executionBody).toContain("JobCommand::ApplySecureRedaction");
    expect(executionBody).toContain("JobCommand::VerifySecureRedaction");
    expect(executionBody).toContain("validate_verification_report");
    expect(executionBody).toContain("atomic_publish_prevalidated_pdf");
    expect(executionBody).not.toMatch(/with_pdfium\(|load_pdf_from_file\(|Document::load\(/);
    const applyVariant = workerSource.match(
      /ApplySecureRedaction\s*\{([\s\S]*?)\n\s*\},\n\s*\/\/\/ Private worker-only verifier/,
    )?.[1];
    const verifyVariant = workerSource.match(
      /VerifySecureRedaction\s*\{([\s\S]*?)\n\s*\},\n\}/,
    )?.[1];
    expect(applyVariant).toContain("output_name:");
    expect(verifyVariant).toContain("transform_report:");
    expect(applyVariant).not.toMatch(/(?:target_path|targetPath|output_path):/);
    expect(verifyVariant).not.toMatch(/(?:target_path|targetPath|output_path):/);
    expect(documentIoSource).toContain("REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH");
    expect(documentIoSource).toContain("REDACTION_TARGET_CONFLICT");

    const executionContract = CONTRACTS.executeSecureRedaction;
    expect(executionContract?.command).toBe("execute_secure_redaction");
    expect(Object.keys(executionContract.payload.required)).toEqual([
      "jobId",
      "sessionId",
      "expectedContentGeneration",
      "policy",
      "acknowledgements",
      "regions",
      "expectedPlanDigest",
      "targetPath",
    ]);

    const executionCallsites = sourceFiles(sourceRoot)
      .filter((path) => readSource(path).includes("startSecureRedactionExecution"))
      .map((path) => relativePosix(sourceRoot, path))
      .filter((path) => !path.endsWith(".test.js") && path !== "services/backend/redaction.js");
    expect(executionCallsites).toEqual(["services/app/secureRedactionController.js"]);

    const controllerSource = readSource(
      resolve(sourceRoot, "services/app/secureRedactionController.js"),
    );
    expect(controllerSource).not.toContain("callBackend");
    expect(controllerSource).not.toContain("execute_secure_redaction");
  });

  it("keeps secure-redaction acceptance seams out of default production builds", () => {
    const redactionSource = readSource(resolve(rustSourceRoot, "redaction.rs"));
    const documentIoSource = readSource(resolve(rustSourceRoot, "document_io.rs"));
    const workerSource = readSource(resolve(rustSourceRoot, "job_worker.rs"));
    const cargoSource = readSource(resolve(rustSourceRoot, "../Cargo.toml"));

    expect(cargoSource).toMatch(/\[features\][\s\S]*?default\s*=\s*\[\]/);
    expect(cargoSource).toContain("secure-redaction-test-support = []");
    expect(redactionSource).toMatch(
      /#\[cfg\(feature = "secure-redaction-test-support"\)\][\s\S]{0,180}pub async fn execute_secure_redaction_core_with_test_support/,
    );
    expect(redactionSource).toMatch(
      /#\[cfg\(feature = "secure-redaction-test-support"\)\][\s\S]{0,180}pub struct SecureRedactionExecutionTestSupport/,
    );
    expect(documentIoSource).toMatch(
      /#\[cfg\(feature = "secure-redaction-test-support"\)\][\s\S]{0,180}atomic_publish_prevalidated_pdf_for_acceptance/,
    );
    expect(redactionSource).not.toMatch(/std::env::(?:var|var_os)\(/);
    expect(workerSource).not.toContain("WHOLE_OBJECT_DESTROY_PROBE");
  });

  it("CLOSURE-3: locks absence of TLS certificate/hostname verification bypass in production AI code", () => {
    const aiFiles = rustSourceFiles(resolve(rustSourceRoot, "ai"));
    const forbiddenTlsPatterns = [
      "danger_accept_invalid_certs",
      "danger_accept_invalid_hostnames",
      "accept_invalid_certs",
      "accept_invalid_hostnames",
      "min_tls_version",
      "tls_built_in_root_certs(false)",
    ];

    for (const file of aiFiles) {
      const isTestFile = file.includes("test") || file.includes("mock");
      if (!isTestFile) {
        const content = readSource(file);
        for (const pattern of forbiddenTlsPatterns) {
          expect(
            content,
            `Production AI file ${relativePosix(rustSourceRoot, file)} must not contain TLS bypass pattern: ${pattern}`,
          ).not.toContain(pattern);
        }
      }
    }
  });
});
