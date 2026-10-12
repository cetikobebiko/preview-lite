import { writable, get } from "svelte/store";

// Geçici (kaydedilmeyen) metin vurguları. Store tek bir document session'a bağlıdır:
// { sessionId, highlights: [{ id, pageIndex, rect: { x, y, width, height }, color }] }
// pageIndex fiziksel sayfa indeksidir; backend sayfaları yeniden numaralandırdığında
// remapHighlightPages ile güncellenir. Farklı session'a geçişte tüm vurgular atılır.
const EMPTY = Object.freeze({ sessionId: null, highlights: [] });

export const annotationsStore = writable(EMPTY);

function isSessionId(value) {
  return typeof value === "string" && value.length > 0;
}

/** Binds the store to the active session; a different session discards every highlight. */
export function bindHighlightSession(sessionId) {
  const next = isSessionId(sessionId) ? sessionId : null;
  if (get(annotationsStore).sessionId === next) return false;
  annotationsStore.set({ sessionId: next, highlights: [] });
  return true;
}

export function addHighlight(sessionId, pageIndex, rect, color) {
  const current = get(annotationsStore);
  if (!isSessionId(sessionId) || current.sessionId !== sessionId) return false;
  if (!Number.isInteger(pageIndex) || pageIndex < 0) return false;
  annotationsStore.set({
    sessionId,
    highlights: [...current.highlights, {
      id: Date.now() + Math.random().toString(36).substr(2, 9),
      pageIndex,
      rect,
      color: color || '#facc15' // Default yellow highlight
    }],
  });
  return true;
}

export function removeHighlight(id) {
  annotationsStore.update(store => ({ ...store, highlights: store.highlights.filter(h => h.id !== id) }));
}

export function clearHighlights(pageIndex) {
  annotationsStore.update(store => ({
    ...store,
    highlights: pageIndex !== undefined ? store.highlights.filter(h => h.pageIndex !== pageIndex) : [],
  }));
}

/**
 * Applies a backend page renumbering. mapPageIndex(oldIndex) returns the new physical
 * index, or -1 when the page no longer exists; highlights on removed pages are dropped.
 */
export function remapHighlightPages(sessionId, mapPageIndex) {
  const current = get(annotationsStore);
  if (!isSessionId(sessionId) || current.sessionId !== sessionId || typeof mapPageIndex !== "function") return false;
  const highlights = [];
  for (const highlight of current.highlights) {
    const pageIndex = mapPageIndex(highlight.pageIndex);
    if (Number.isInteger(pageIndex) && pageIndex >= 0) highlights.push({ ...highlight, pageIndex });
  }
  annotationsStore.set({ sessionId, highlights });
  return true;
}

/** Normalized rect after the page itself is rotated clockwise by a multiple of 90 degrees. */
export function rotateNormalizedRect(rect, degrees) {
  const { x, y, width, height } = rect;
  switch (((degrees % 360) + 360) % 360) {
    case 90: return { x: 1 - (y + height), y: x, width: height, height: width };
    case 180: return { x: 1 - (x + width), y: 1 - (y + height), width, height };
    case 270: return { x: y, y: 1 - (x + width), width: height, height: width };
    default: return { x, y, width, height };
  }
}

/** Keeps highlights on their text after the page's /Rotate changes by `degrees` (clockwise). */
export function rotateHighlightPage(sessionId, pageIndex, degrees) {
  const current = get(annotationsStore);
  if (!isSessionId(sessionId) || current.sessionId !== sessionId || !Number.isInteger(degrees) || degrees % 90 !== 0) return false;
  annotationsStore.set({
    sessionId,
    highlights: current.highlights.map((highlight) => highlight.pageIndex === pageIndex
      ? { ...highlight, rect: rotateNormalizedRect(highlight.rect, degrees) }
      : highlight),
  });
  return true;
}

export function highlightsForPage(store, sessionId, pageIndex) {
  if (!store || !isSessionId(sessionId) || store.sessionId !== sessionId) return [];
  return store.highlights.filter(h => h.pageIndex === pageIndex);
}

export function resetHighlightsForTests() {
  annotationsStore.set(EMPTY);
}
