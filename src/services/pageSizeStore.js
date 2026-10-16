import { getPageSizes } from "./backend/viewer.js";
import { normalizePageSizes } from "../utils/pageGeometry.js";

/**
 * Per-document page sizes for placeholder heights.
 *
 * Every PdfPage of a document asks for the same (sessionId, contentGeneration) key, so
 * the worker is asked once per key. Only the latest key is kept: a new session or content
 * generation drops the previous sizes, and a late answer for an old key is ignored.
 * A failed load resolves to null and is not retried for that key; pages then fall back
 * to estimated sizes, and the page layout model (pageLayoutModel.js) has no layout.
 */
export function createPageSizeStore({ load = getPageSizes } = {}) {
  let currentKey = "";
  let sizes = null;
  let pending = null;

  const keyOf = (sessionId, contentGeneration) =>
    sessionId ? `${sessionId}:${contentGeneration}` : "";

  function peek(sessionId, contentGeneration, pageIndex) {
    const key = keyOf(sessionId, contentGeneration);
    if (!key || key !== currentKey || !sizes) return null;
    return sizes[pageIndex] ?? null;
  }

  /** All loaded sizes of the document, or null until they are loaded. */
  function peekAll(sessionId, contentGeneration) {
    const key = keyOf(sessionId, contentGeneration);
    return key && key === currentKey ? sizes : null;
  }

  async function request(sessionId, contentGeneration) {
    const key = keyOf(sessionId, contentGeneration);
    if (!key) return null;
    if (key === currentKey) return sizes ?? pending;
    currentKey = key;
    sizes = null;
    const loading = Promise.resolve()
      .then(() => load(sessionId, contentGeneration))
      .then(
        (raw) => {
          if (currentKey !== key) return null;
          sizes = normalizePageSizes(raw);
          return sizes;
        },
        (error) => {
          // Not retried for the same key: every mounted page would repeat the failing call.
          if (currentKey === key) {
            console.warn("Sayfa boyutları alınamadı; tahmini yükseklik kullanılacak:", error);
          }
          return null;
        },
      )
      .finally(() => {
        if (pending === loading) pending = null;
      });
    pending = loading;
    return loading;
  }

  async function requestPage(sessionId, contentGeneration, pageIndex) {
    const loaded = await request(sessionId, contentGeneration);
    return Array.isArray(loaded) ? loaded[pageIndex] ?? null : null;
  }

  return Object.freeze({ peek, peekAll, request, requestPage });
}

export const pageSizeStore = createPageSizeStore();
