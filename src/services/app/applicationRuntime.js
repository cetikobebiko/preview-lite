import { getCurrentWindow } from "../platform/window.js";
import { createApplicationLifecycleController } from "./applicationLifecycleController.js";
import { createKeyboardController } from "./keyboardController.js";
import { createWindowLifecycleController } from "./windowLifecycleController.js";

/** Owns global startup, input routing and deterministic mount/unmount ordering. */
export function createApplicationRuntime({
  viewer,
  ui,
  documentSession,
  asyncPrompt,
  presentationController,
  documentRuntime,
  viewerRuntime,
  featureRuntime,
  callbacks,
  getViewerArea,
  setMounted,
  windowApi = getCurrentWindow(),
  createLifecycleController = createApplicationLifecycleController,
  createWindowController = createWindowLifecycleController,
  createKeyboard = createKeyboardController,
}) {
  const lifecycleController = createLifecycleController({
    viewer,
    ui,
    onOpenFile: documentRuntime.openFile,
    onToast: presentationController.showToast,
    onError: (message, error) => console.warn(message, error),
    isDocumentOpen: () => Boolean(documentSession.active || viewer.activeImageSourcePath),
  });
  callbacks.saveSettings = lifecycleController.saveViewerSettings;

  const transitions = documentRuntime.transitionApi;
  const windowLifecycleController = createWindowController({
    windowApi,
    documentOpenCoordinator: documentRuntime.coordinator,
    getActiveSessionId: () => documentSession.activeSessionId,
    beginSessionTransition: transitions.beginSessionTransition,
    chooseTransition: documentRuntime.chooseDocumentTransition,
    saveTransition: transitions.saveTransition,
    discardTransition: transitions.discardTransition,
    cancelTransition: transitions.cancelTransition,
    closeSession: (sessionId) => documentSession.closeSession(sessionId),
    closeAllDocumentSessions: transitions.closeAllDocumentSessions,
    onError: (error) => presentationController.setError("Pencere kapatılamadı: " + error.toString()),
  });

  const keyboardController = createKeyboard({
    hasDocument: () => Boolean(documentRuntime.activeDisplayPath || viewer.activeImageSourcePath),
    isEditableTarget: () => {
      const active = globalThis.document?.activeElement;
      const tag = active?.tagName?.toLowerCase();
      return tag === "input" || tag === "textarea" || active?.isContentEditable;
    },
    save: documentRuntime.save,
    saveAs: documentRuntime.saveAs,
    toggleSidebar: () => ui.togglePanel("sidebar"),
    openFile: documentRuntime.selectFile,
    toggleCommandPalette: () => featureRuntime.runUiAction("command_palette", () => {
      if (ui.isModalOpen("command_palette")) ui.closeModal("command_palette");
      else ui.openModal("command_palette");
    }),
    openSearch: () => featureRuntime.runUiAction("search", () => ui.setPanel("search", true)),
    openExport: () => ui.openModal("export"),
    requestPage: async () => {
      const page = await asyncPrompt.request({
        title: "Sayfaya Git",
        label: "Gitmek istediğiniz sayfa numarası",
        placeholder: `1-${viewer.pageOrder.length}`,
      });
      const index = Number.parseInt(page, 10) - 1;
      if (Number.isInteger(index) && index >= 0 && index < viewer.pageOrder.length) {
        await viewerRuntime.navigation.jumpToPage(index);
      }
    },
    goToPage: viewerRuntime.navigation.jumpToPage,
    adjustZoom: viewerRuntime.navigation.adjustZoom,
    toggleShortcuts: () => {
      if (ui.isModalOpen("shortcuts")) ui.closeModal("shortcuts");
      else ui.openModal("shortcuts");
    },
    nextPage: viewerRuntime.navigation.goToNextPage,
    previousPage: viewerRuntime.navigation.goToPrevPage,
  });

  function mount() {
    setMounted(true);
    featureRuntime.start();
    void lifecycleController.start()
      .catch((error) => console.error("Uygulama yaşam döngüsü başlatılamadı:", error));
    void windowLifecycleController.start()
      .catch((error) => console.error("Close listener kurulamadı:", error));
    viewerRuntime.start(getViewerArea());

    return () => {
      setMounted(false);
      const closedGeneration = documentSession.contentGeneration;
      const coordinatorStop = documentRuntime.coordinator.stop();
      featureRuntime.destroy();
      documentRuntime.lifecycleController.destroy();
      lifecycleController.stop();
      windowLifecycleController.destroy();
      asyncPrompt.cancel();
      presentationController.destroy();
      viewerRuntime.stop();
      void coordinatorStop.then((result) => {
        if (result.status !== "stopped") return;
        void viewerRuntime.deactivatePdf(closedGeneration).catch(() => {
          console.warn("Render document deactivation failed.");
        });
      });
    };
  }

  return Object.freeze({
    mount,
    handleKeyDown: keyboardController.handleKeyDown,
    saveViewerSettings: lifecycleController.saveViewerSettings,
    windowLifecycleController,
  });
}
