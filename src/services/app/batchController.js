import { listen as tauriListen } from "@tauri-apps/api/event";
import { ask as askDialog } from "@tauri-apps/plugin-dialog";
import { guardFeatureAction } from "../../featureActionGuard.js";
import { getRuntimeFeatureContext } from "../../featureRegistry.js";
import {
  cancelBatch as cancelBatchBackend,
  getBatchStatus as getBatchStatusBackend,
  revealBatchOutputDir as revealBatchOutputDirBackend,
  scanBatchInputs as scanBatchInputsBackend,
  startBatch as startBatchBackend,
} from "../backend/batch.js";

export const BATCH_PROGRESS_EVENT = "batch_progress";

const guarded = (callback) =>
  guardFeatureAction("batch_processing", getRuntimeFeatureContext(), callback);

/** Batch commands behind the `batch_processing` feature gate, for UI components. */
export const batchActions = Object.freeze({
  scan: (paths, recursive) => guarded(() => scanBatchInputsBackend(paths, recursive)),
  start: (request) => guarded(() => startBatchBackend(request)),
  cancel: (batchId) => guarded(() => cancelBatchBackend(batchId)),
  reveal: (batchId) => guarded(() => revealBatchOutputDirBackend(batchId)),
});

/**
 * Follows one batch for a mounted wizard. Subscribes before reading the status,
 * so no event is lost: events at or below the status' `lastSequence` are
 * already in it and are dropped, later ones are applied in order. Events of
 * other batches are ignored; a gap in the sequence re-reads the status.
 */
export function createBatchTracker({
  onChange = () => {},
  listen = tauriListen,
  getBatchStatus = getBatchStatusBackend,
} = {}) {
  let status = null;
  let batchId = null;
  let unlisten = null;
  let disposed = false;
  let pending = [];
  let syncing = false;

  function publish() {
    onChange(status);
  }

  function apply(event) {
    if (!status || event.batchId !== status.batchId) return;
    if (event.sequence <= status.lastSequence) return;
    if (event.sequence !== status.lastSequence + 1) {
      void resync();
      return;
    }
    if (event.terminal && event.status) {
      status = event.status;
    } else {
      const files = event.file
        ? status.files.map((file) => (file.index === event.file.index ? event.file : file))
        : status.files;
      status = { ...status, files, counts: event.counts, lastSequence: event.sequence };
    }
    publish();
  }

  function handle(event) {
    if (disposed || !event || event.batchId !== batchId) return;
    if (syncing) pending.push(event);
    else apply(event);
  }

  async function resync() {
    if (disposed || !batchId || syncing) return;
    syncing = true;
    try {
      const fresh = await getBatchStatus(batchId);
      if (disposed) return;
      if (fresh) status = fresh;
      publish();
    } finally {
      syncing = false;
      const queued = pending.sort((left, right) => left.sequence - right.sequence);
      pending = [];
      for (const event of queued) apply(event);
    }
  }

  async function ensureListening() {
    if (unlisten || disposed) return;
    const stop = await listen(BATCH_PROGRESS_EVENT, (message) => handle(message.payload));
    if (disposed) stop();
    else unlisten = stop;
  }

  /** Follows `id`; with `null`, adopts the backend's current or last batch. */
  async function attach(id = null) {
    batchId = id;
    await ensureListening();
    syncing = true;
    try {
      const fresh = await getBatchStatus(id);
      if (disposed) return null;
      status = fresh ?? null;
      batchId = status?.batchId ?? null;
      publish();
    } finally {
      syncing = false;
      const queued = pending
        .filter((event) => event.batchId === batchId)
        .sort((left, right) => left.sequence - right.sequence);
      pending = [];
      for (const event of queued) apply(event);
    }
    return status;
  }

  function dispose() {
    disposed = true;
    pending = [];
    unlisten?.();
    unlisten = null;
  }

  return Object.freeze({
    attach,
    dispose,
    current: () => status,
    // Events for a batch whose id is not known yet (between start and attach) are dropped,
    // which is safe because `attach` reads the full status after subscribing.
    handle,
  });
}

/** True while the backend reports a running batch; used before the window closes. */
export async function runningBatchId(getBatchStatus = getBatchStatusBackend) {
  try {
    const status = await getBatchStatus(null);
    return status?.state === "running" ? status.batchId : null;
  } catch {
    return null;
  }
}

/**
 * Before the window closes: with a running batch, asks whether to stop it.
 * Yes cancels it (finished files keep their outputs); no keeps the window open.
 */
export async function confirmRunningBatchStop({
  getBatchStatus = getBatchStatusBackend,
  cancelBatch = cancelBatchBackend,
  ask = askDialog,
} = {}) {
  const batchId = await runningBatchId(getBatchStatus);
  if (!batchId) return true;
  const confirmed = await ask(
    "Bir toplu işlem sürüyor. Uygulama kapanırsa iptal edilir; tamamlanan dosyaların çıktıları kalır. Kapatılsın mı?",
    { title: "Toplu işlem sürüyor", kind: "warning" },
  );
  if (!confirmed) return false;
  try {
    await cancelBatch(batchId);
  } catch {
    // The backend also cancels a running batch while the app exits.
  }
  return true;
}

export const BATCH_OPERATIONS = Object.freeze([
  Object.freeze({ id: "optimize", label: "Dosya boyutunu küçült" }),
  Object.freeze({ id: "export_document", label: "Belge olarak dışa aktar" }),
  Object.freeze({ id: "export_text", label: "Metin olarak dışa aktar" }),
  Object.freeze({ id: "export_images", label: "Sayfaları görsel olarak dışa aktar" }),
  Object.freeze({ id: "security", label: "Parola korumalı ya da korumasız kopya" }),
]);

export const DOCUMENT_FORMATS = Object.freeze([
  Object.freeze({ value: "docx", label: "Word (.docx)" }),
  Object.freeze({ value: "pptx", label: "PowerPoint (.pptx)" }),
  Object.freeze({ value: "html", label: "HTML (.html)" }),
  Object.freeze({ value: "rtf", label: "RTF (.rtf)" }),
  Object.freeze({ value: "md", label: "Markdown (.md)" }),
]);

export const TEXT_FORMATS = Object.freeze([
  Object.freeze({ value: "txt", label: "Düz metin (.txt)" }),
  Object.freeze({ value: "md", label: "Markdown (.md)" }),
]);

export const IMAGE_FORMATS = Object.freeze([
  Object.freeze({ value: "png", label: "PNG" }),
  Object.freeze({ value: "jpeg", label: "JPEG" }),
]);

export const CONFLICT_POLICIES = Object.freeze([
  Object.freeze({ value: "number", label: "Numara ekle (rapor (2).pdf)" }),
  Object.freeze({ value: "skip", label: "Dosyayı atla" }),
  Object.freeze({ value: "overwrite", label: "Üzerine yaz" }),
]);

const FILE_STATE_LABELS = Object.freeze({
  pending: "Bekliyor",
  running: "İşleniyor",
  completed: "Tamamlandı",
  skipped: "Atlandı",
  failed: "Hata",
  cancelled: "İptal edildi",
});

export const fileStateLabel = (state) => FILE_STATE_LABELS[state] ?? state;

// Optimize and security copies rewrite each PDF; a signed input's signatures are not
// valid in its output. The backend reports this per completed file.
export const BATCH_SIGNATURE_NOTE =
  "İmzalı belgelerin kopyası baştan yazılır; dijital imzalar kopyada geçersiz olur. Girdi dosyaları değişmez.";
export const BATCH_SIGNATURE_LOSS_TEXT = "Dijital imzalar çıktıda geçersiz.";

/** Number of completed files whose signatures are not valid in the output. */
export const signatureLossCount = (status) =>
  (status?.files ?? []).filter((file) => file.signaturesInvalidated).length;

/** The wire `operation` for the wizard's choices. */
export function buildBatchOperation(kind, options) {
  switch (kind) {
    case "optimize":
      return { kind, settings: options.optimizeSettings };
    case "export_document":
      return { kind, format: options.documentFormat };
    case "export_text":
      return { kind, format: options.textFormat };
    case "export_images":
      return { kind, format: options.imageFormat, dpi: Number(options.dpi) };
    case "security":
      return { kind, op: options.securityOp };
    default:
      throw new TypeError(`Unknown batch operation: ${kind}`);
  }
}

/** Plain-text report for the clipboard; never includes operation settings. */
export function batchReportText(status, operationLabel = status.operation) {
  const counts = status.counts;
  const lines = [
    `Toplu işlem: ${operationLabel}`,
    `Çıktı klasörü: ${status.outputDir}`,
    `Tamamlandı: ${counts.completed}, atlandı: ${counts.skipped}, hata: ${counts.failed}, iptal: ${counts.cancelled}`,
  ];
  const signed = signatureLossCount(status);
  if (signed > 0) lines.push(`Dijital imzaları çıktıda geçersiz olan dosya: ${signed}`);
  lines.push("");
  for (const file of status.files) {
    const reason = file.reason ? ` — ${file.reason.message} (${file.reason.code})` : "";
    lines.push(`${fileStateLabel(file.state)}: ${file.inputPath}${reason}`);
    if (file.signaturesInvalidated) lines.push(`  ${BATCH_SIGNATURE_LOSS_TEXT}`);
    for (const output of file.outputs) lines.push(`  → ${output}`);
  }
  for (const warning of status.warnings ?? []) lines.push(`Uyarı: ${warning}`);
  return lines.join("\n");
}
