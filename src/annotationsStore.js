import { writable } from "svelte/store";

// Vurgulama modeli: { id, pageIndex, rect: { x, y, width, height }, color }
export const annotationsStore = writable([]);

export function addHighlight(pageIndex, rect, color) {
  annotationsStore.update(store => {
    return [...store, {
      id: Date.now() + Math.random().toString(36).substr(2, 9),
      pageIndex,
      rect,
      color: color || '#facc15' // Default yellow highlight
    }];
  });
}

export function removeHighlight(id) {
  annotationsStore.update(store => store.filter(h => h.id !== id));
}

export function clearHighlights(pageIndex) {
  if (pageIndex !== undefined) {
    annotationsStore.update(store => store.filter(h => h.pageIndex !== pageIndex));
  } else {
    annotationsStore.set([]);
  }
}
