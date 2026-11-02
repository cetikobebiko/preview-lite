/**
 * Old-to-new physical page index mappings for backend page mutations. Each returns
 * mapPageIndex(oldIndex) -> newIndex, or -1 when the old page no longer exists.
 */
export function pageMappingAfterDelete(deletedIndices) {
  const deleted = [...new Set(deletedIndices)].filter((index) => Number.isInteger(index) && index >= 0).sort((a, b) => a - b);
  return (oldIndex) => {
    if (!Number.isInteger(oldIndex) || oldIndex < 0 || deleted.includes(oldIndex)) return -1;
    return oldIndex - deleted.filter((index) => index < oldIndex).length;
  };
}

/**
 * After `insertedCount` pages went in at `atIndex` and the `replacedCount` pages that were there
 * were removed: old pages before `atIndex` keep their index, replaced ones map to -1.
 */
export function pageMappingAfterImport(atIndex, insertedCount, replacedCount = 0) {
  return (oldIndex) => {
    if (!Number.isInteger(oldIndex) || oldIndex < 0) return -1;
    if (oldIndex < atIndex) return oldIndex;
    if (oldIndex < atIndex + replacedCount) return -1;
    return oldIndex + insertedCount - replacedCount;
  };
}

/** Re-keys a page-keyed drawing map ({ [pageIndex]: [{ page_index, ... }] }); removed pages are dropped. */
export function remapPageKeyedDrawings(annotationsMap, mapPageIndex) {
  const remapped = {};
  for (const [key, items] of Object.entries(annotationsMap ?? {})) {
    const pageIndex = mapPageIndex(Number(key));
    if (!Number.isInteger(pageIndex) || pageIndex < 0 || !items?.length) continue;
    remapped[pageIndex] = [...(remapped[pageIndex] ?? []), ...items.map((item) => ({ ...item, page_index: pageIndex }))];
  }
  return remapped;
}

/** newOrder[newIndex] = oldIndex, a full permutation as enforced by the backend. */
export function pageMappingAfterReorder(newOrder) {
  const newIndexByOld = new Map();
  if (Array.isArray(newOrder)) newOrder.forEach((oldIndex, newIndex) => newIndexByOld.set(oldIndex, newIndex));
  return (oldIndex) => newIndexByOld.get(oldIndex) ?? -1;
}
