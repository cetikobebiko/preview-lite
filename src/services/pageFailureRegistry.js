import { writable } from "svelte/store";

export const DOCUMENT_QUARANTINED = "PDFIUM_DOCUMENT_QUARANTINED";
export const PAGE_QUARANTINED = "PDFIUM_PAGE_QUARANTINED";
export const PAGE_SURFACES = Object.freeze(["render", "thumbnail", "textLayout"]);

// Worker failures after which another automatic request for the same page is
// pointless or harmful: every repeated timeout, memory-limit kill or crash
// kills the shared PDF worker for all open documents. Cancellations and ordinary errors are not
// tracked, so they keep the existing per-request behaviour.
const SUPPRESSED_CODES = new Set([
  "PDFIUM_WORKER_TIMEOUT",
  "PDFIUM_WORKER_MEMORY_LIMIT",
  "PDFIUM_WORKER_CRASHED",
  PAGE_QUARANTINED,
  DOCUMENT_QUARANTINED,
]);

// Sessions whose entries are kept; older ones are dropped first.
const MAX_DOCUMENTS = 8;

/**
 * Remembers which page surfaces of a document generation failed with a
 * worker-safety error, so scrolling, zooming or re-mounting does not send the
 * same request again. Entries end with the generation or the session (a
 * reopened document is a new session); an explicit retry clears a page.
 * The registry is a Svelte store whose value changes on every update.
 */
export function createPageFailureRegistry() {
  const documents = new Map();
  const version = writable(0);
  const bump = () => version.update((value) => value + 1);

  function documentEntry(sessionId, contentGeneration, create) {
    if (!sessionId) return null;
    const key = `${sessionId}:${contentGeneration}`;
    let entry = documents.get(key);
    if (!entry && create) {
      for (const [otherKey, other] of documents) {
        if (other.sessionId === sessionId) documents.delete(otherKey);
      }
      entry = { sessionId, quarantined: false, pages: new Map() };
      documents.set(key, entry);
      while (documents.size > MAX_DOCUMENTS) {
        documents.delete(documents.keys().next().value);
      }
    }
    return entry ?? null;
  }

  /** Records `error` when it is a worker-safety failure; returns whether it did. */
  function record({ sessionId, contentGeneration, pageIndex, surface }, error) {
    const code = error?.code;
    if (!SUPPRESSED_CODES.has(code) || !PAGE_SURFACES.includes(surface)) return false;
    const entry = documentEntry(sessionId, contentGeneration, true);
    if (!entry) return false;
    if (code === DOCUMENT_QUARANTINED) {
      entry.quarantined = true;
    } else {
      if (!entry.pages.has(pageIndex)) entry.pages.set(pageIndex, new Map());
      entry.pages.get(pageIndex).set(surface, code);
    }
    bump();
    return true;
  }

  /** The failure code that blocks this surface, or null. */
  function failure({ sessionId, contentGeneration, pageIndex, surface }) {
    const entry = documentEntry(sessionId, contentGeneration, false);
    if (!entry) return null;
    if (entry.quarantined) return DOCUMENT_QUARANTINED;
    return entry.pages.get(pageIndex)?.get(surface) ?? null;
  }

  function isDocumentQuarantined(sessionId, contentGeneration) {
    return documentEntry(sessionId, contentGeneration, false)?.quarantined === true;
  }

  /** Clears the given surfaces of a page (all of them by default). */
  function clearPage(
    { sessionId, contentGeneration, pageIndex },
    surfaces = PAGE_SURFACES,
  ) {
    const pages = documentEntry(sessionId, contentGeneration, false)?.pages;
    const failures = pages?.get(pageIndex);
    if (!failures) return false;
    let cleared = false;
    for (const surface of surfaces) cleared = failures.delete(surface) || cleared;
    if (failures.size === 0) pages.delete(pageIndex);
    if (cleared) bump();
    return cleared;
  }

  function reset() {
    documents.clear();
    bump();
  }

  return Object.freeze({
    subscribe: version.subscribe,
    record,
    failure,
    isDocumentQuarantined,
    clearPage,
    reset,
  });
}

export const pageFailureRegistry = createPageFailureRegistry();

export function pageFailureMessage(code, pageIndex) {
  const page = `Sayfa ${pageIndex + 1}`;
  switch (code) {
    case "PDFIUM_WORKER_TIMEOUT":
      return `${page} işlenirken zaman aşımı oluştu.`;
    case "PDFIUM_WORKER_MEMORY_LIMIT":
      return `${page} işlenirken PDF motoru bellek sınırını aştığı için durduruldu.`;
    case "PDFIUM_WORKER_CRASHED":
      return `${page} işlenirken PDF motoru beklenmedik şekilde durdu.`;
    case PAGE_QUARANTINED:
      return `${page} tekrarlanan zaman aşımı ya da bellek aşımı nedeniyle askıya alındı.`;
    case DOCUMENT_QUARANTINED:
      return "Belge PDF motorunu tekrar tekrar durdurduğu için askıya alındı.";
    default:
      return "";
  }
}
