export function createUiState() {
  const state = $state({ activeModal: null, panels: { sidebar: true, search: false, a11y: false, prepress: false, sensitiveDataScan: false, documentCompare: false, secureRedaction: false, readingRuler: false, markup: false }, settingsPopover: false, isDraggingFileOver: false, toast: "", error: "" });
  const api = {
    get activeModal() { return state.activeModal; },
    openModal(type, props = {}) { state.activeModal = { type, props }; },
    closeModal(type = null) { if (!type || state.activeModal?.type === type) state.activeModal = null; },
    isModalOpen(type) { return state.activeModal?.type === type; },
    get panels() { return state.panels; },
    togglePanel(name) { state.panels[name] = !state.panels[name]; },
    setPanel(name, value) { state.panels[name] = value; },
    get settingsPopover() { return state.settingsPopover; }, set settingsPopover(value) { state.settingsPopover = value; },
    get isDraggingFileOver() { return state.isDraggingFileOver; }, set isDraggingFileOver(value) { state.isDraggingFileOver = value; },
    get toast() { return state.toast; }, set toast(value) { state.toast = value; },
    get error() { return state.error; }, set error(value) { state.error = value; },
  };
  const modalNames = {
    showShortcuts: "shortcuts", showOrganizer: "page_organizer", showMergeSplit: "merge_split",
    isCmdPaletteOpen: "command_palette", isPluginManagerOpen: "plugin_manager",
    isAiModalOpen: "ai_assistant",
    isAiSettingsOpen: "ai_settings", isBionicModalOpen: "bionic", isAnalyticsModalOpen: "analytics",
    isMetadataModalOpen: "metadata", isPasswordModalOpen: "password", isExportModalOpen: "export",
  };
  for (const [property, modal] of Object.entries(modalNames)) {
    Object.defineProperty(api, property, {
      enumerable: true,
      get: () => state.activeModal?.type === modal,
      set: (value) => { if (value) state.activeModal = { type: modal, props: {} }; else if (state.activeModal?.type === modal) state.activeModal = null; },
    });
  }
  Object.defineProperties(api, {
    showSidebar: { get: () => state.panels.sidebar, set: (value) => { state.panels.sidebar = value; } },
    isSearchBarOpen: { get: () => state.panels.search, set: (value) => { state.panels.search = value; } },
    isA11yPanelOpen: { get: () => state.panels.a11y, set: (value) => { state.panels.a11y = value; } },
    isPrepressPanelOpen: { get: () => state.panels.prepress, set: (value) => { state.panels.prepress = value; } },
    isSensitiveDataScanPanelOpen: { get: () => state.panels.sensitiveDataScan, set: (value) => { state.panels.sensitiveDataScan = value; } },
    isSecureRedactionPanelOpen: { get: () => state.panels.secureRedaction, set: (value) => { state.panels.secureRedaction = value; } },
    isReadingRulerActive: { get: () => state.panels.readingRuler, set: (value) => { state.panels.readingRuler = value; } },
  });
  return api;
}
