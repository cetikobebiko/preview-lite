/** Owns keyboard routing, while injected callbacks keep state ownership in stores. */
export function createKeyboardController({
  hasDocument, isEditableTarget, save, saveAs, toggleSidebar, openFile,
  toggleCommandPalette, openSearch, openExport, requestPage, goToPage,
  adjustZoom, toggleShortcuts, nextPage, previousPage,
} = {}) {
  async function handleKeyDown(event) {
    const modifier = event.metaKey || event.ctrlKey;
    const key = event.key?.toLowerCase();
    const handled = (action) => { event.preventDefault(); return action(); };
    if (modifier && key === "s") return handled(() => event.shiftKey ? saveAs() : save());
    if (modifier && key === "b") return handled(toggleSidebar);
    if (modifier && key === "o") return handled(openFile);
    if (!hasDocument()) return false;
    if (modifier && key === "k") return handled(toggleCommandPalette);
    if (modifier && key === "f") return handled(openSearch);
    if (modifier && key === "p") return handled(openExport);
    if (modifier && key === "g") return handled(requestPage);
    if (modifier && event.key === "=") return handled(() => adjustZoom(0.1));
    if (modifier && event.key === "-") return handled(() => adjustZoom(-0.1));
    if (modifier && event.key === "0") return handled(() => adjustZoom(null));
    if (modifier && event.key === "/") return handled(toggleShortcuts);
    if (isEditableTarget()) return false;
    if (event.key === "?") return handled(toggleShortcuts);
    if (["ArrowRight", "ArrowDown", "PageDown"].includes(event.key)) return handled(nextPage);
    if (["ArrowLeft", "ArrowUp", "PageUp"].includes(event.key)) return handled(previousPage);
    return false;
  }
  return Object.freeze({ handleKeyDown });
}
