import { isSignatureLossDeclined } from "../signatureLossGuard.js";
import { pageMappingAfterImport } from "./pageIndexMapping.js";

export const CONTINUITY_SCAN_FEATURE_ID = "macos_continuity_camera";
export const CONTINUITY_SCAN_MODAL = "continuity_scan";
export const CONTINUITY_SCAN_SIGNED_EDIT_LABEL = "iPhone'dan sayfa ekleme";

const DEVICE_REQUIREMENTS =
  "iPhone veya iPad aynı Apple hesabıyla oturum açmış, Wi-Fi ve Bluetooth açık ve Mac'e yakın olmalıdır.";

const ERROR_MESSAGES = Object.freeze({
  CONTINUITY_SCAN_NO_DEVICE: `Continuity Camera için uygun bir iPhone veya iPad bulunamadı. ${DEVICE_REQUIREMENTS}`,
  CONTINUITY_SCAN_TIMEOUT: "iPhone/iPad'den zamanında içerik gelmedi; tarama durduruldu ve belge değişmedi.",
  CONTINUITY_SCAN_UNSUPPORTED_CONTENT: "Taramadan desteklenmeyen bir içerik geldi; belge değişmedi.",
  CONTINUITY_SCAN_LIMIT_EXCEEDED: "Tarama sınırları aşıldı (en fazla 100 sayfa, toplam 512 MB); belge değişmedi.",
  CONTINUITY_SCAN_BUSY: "Başka bir iPhone/iPad taraması sürüyor.",
  STALE_GENERATION: "Belge tarama beklenirken değişti; tarama eklenmedi.",
  FEATURE_UNAVAILABLE: "iPhone/iPad ile tarama bu sistemde kullanılamıyor (macOS 10.14 veya üstü gerekir).",
});

/** User-facing message for a failed scan; the document is unchanged in every case. */
export function continuityScanErrorMessage(error) {
  const known = ERROR_MESSAGES[error?.code];
  if (known) return known;
  const detail = error?.message ?? String(error);
  return `iPhone/iPad taraması eklenemedi; belge değişmedi. ${detail}`;
}

/**
 * Owns the macOS Continuity Camera scan: one scan at a time, a cancellable waiting dialog,
 * and stale-result protection. The backend runs the helper as a session job and inserts the
 * result as one undoable mutation; this controller only starts, cancels and applies it.
 */
export function createContinuityScanController({
  getContext,
  guard,
  featureContext,
  reportUnavailable,
  confirmSignedEdit,
  resolvePageFit,
  startScan,
  getPageCount,
  applyMutationSnapshot,
  ui,
  onToast,
  onError,
}) {
  let active = null;

  async function scan(options = {}) {
    try {
      return await guard(CONTINUITY_SCAN_FEATURE_ID, featureContext, () => execute(options));
    } catch (error) {
      if (!reportUnavailable(error)) throw error;
      return null;
    }
  }

  async function execute({ atIndex: requestedIndex } = {}) {
    if (active) {
      onToast(ERROR_MESSAGES.CONTINUITY_SCAN_BUSY);
      return null;
    }
    const context = getContext();
    if (!context.isPdf || !context.sessionId) {
      onToast("iPhone/iPad ile tarayıp eklemek için açık bir PDF gerekir.");
      return null;
    }
    const { sessionId, contentGeneration } = context;
    const pageCount = Math.max(0, context.pageCount ?? 0);
    const atIndex = Number.isInteger(requestedIndex)
      ? Math.min(Math.max(requestedIndex, 0), pageCount)
      : context.pageIndex >= 0
        ? Math.min(context.pageIndex + 1, pageCount)
        : pageCount;
    const neighborIndex = Math.min(Math.max(atIndex - 1, 0), Math.max(pageCount - 1, 0));
    const scanState = { sessionId, job: null, cancelled: false };
    active = scanState;
    try {
      await confirmSignedEdit(sessionId, contentGeneration, CONTINUITY_SCAN_SIGNED_EDIT_LABEL);
      const pageFit = await resolvePageFit(sessionId, contentGeneration, neighborIndex);
      if (scanState.cancelled) return null;
      scanState.job = startScan(sessionId, contentGeneration, atIndex, pageFit, featureContext);
      ui.openModal(CONTINUITY_SCAN_MODAL, { oncancel: cancel });
      const { snapshot } = await scanState.job.promise;
      const newCount = await getPageCount(snapshot.id, snapshot.contentGeneration);
      const added = Math.max(0, newCount - pageCount);
      await applyMutationSnapshot(snapshot, { mapPageIndex: pageMappingAfterImport(atIndex, added, 0) });
      if (getContext().sessionId === snapshot.id) {
        onToast(`${added} sayfa iPhone/iPad'den eklendi (${atIndex + 1}. sayfadan itibaren). Geri almak için ⌘Z.`);
      }
      return snapshot;
    } catch (error) {
      if (isSignatureLossDeclined(error)) return null;
      if (error?.code === "SESSION_JOB_CANCELLED") {
        onToast("iPhone/iPad taraması iptal edildi; belge değişmedi.");
        return null;
      }
      if (reportUnavailable(error)) return null;
      onError(continuityScanErrorMessage(error));
      return null;
    } finally {
      if (active === scanState) active = null;
      ui.closeModal(CONTINUITY_SCAN_MODAL);
    }
  }

  /** Cancels the running scan; the backend kills the helper and leaves the document as is. */
  function cancel() {
    const scanState = active;
    if (!scanState) return;
    scanState.cancelled = true;
    void scanState.job?.cancel?.()?.catch?.((error) => console.warn("Tarama iptali gönderilemedi:", error));
  }

  /** A scan belongs to the document it started on; switching or closing it cancels the scan. */
  function reconcileContext() {
    if (active && getContext().sessionId !== active.sessionId) cancel();
  }

  return Object.freeze({
    scan,
    cancel,
    reconcileContext,
    get isActive() {
      return active !== null;
    },
  });
}
