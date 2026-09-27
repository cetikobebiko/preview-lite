import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { flushSync } from "svelte";

const appRuntime = vi.hoisted(() => {
  const cleanup = vi.fn();
  const mount = vi.fn(() => cleanup);
  return { cleanup, mount };
});

vi.mock("./services/app/applicationRuntime.js", () => ({
  createApplicationRuntime: vi.fn(() => ({
    mount: appRuntime.mount,
    handleKeyDown: vi.fn(),
  })),
}));

vi.mock("./services/app/documentRuntime.js", () => ({
  createDocumentRuntime: vi.fn(() => ({
    isPdf: false,
    activeDisplayPath: "",
    fileName: "Dosya",
    isLoadedPdfDocument: false,
    isDirty: false,
    savePosition: vi.fn(),
    selectFile: vi.fn(),
    save: vi.fn(),
    applyBackendSessionSnapshot: vi.fn(),
    applyMutationSnapshot: vi.fn(),
    lifecycleController: { submitPassword: vi.fn(), cancelPassword: vi.fn() },
  })),
}));

vi.mock("./services/app/viewerRuntime.js", () => ({
  createViewerRuntime: vi.fn(() => ({
    controller: {},
    navigation: {
      jumpToPage: vi.fn(), handlePageIntersect: vi.fn(), goBack: vi.fn(), goForward: vi.fn(),
      goToPrevPage: vi.fn(), goToNextPage: vi.fn(), handlePageInputChange: vi.fn(),
      zoomOut: vi.fn(), resetZoom: vi.fn(), zoomIn: vi.fn(), toggleSinglePageMode: vi.fn(),
      toggleBookMode: vi.fn(),
    },
    edits: { rotateLeft: vi.fn(), rotateRight: vi.fn(), toggleFlipH: vi.fn(), toggleFlipV: vi.fn() },
    annotationCanvas: { handleMouseDown: vi.fn(), handleMouseMove: vi.fn(), handleMouseUp: vi.fn() },
  })),
}));

vi.mock("./services/app/featureRuntime.js", () => ({
  createFeatureRuntime: vi.fn(() => ({
    featureUi: () => ({ visible: false, runnable: false, warning: "" }),
    featureControlLabel: (_id, label) => label,
    runUiAction: vi.fn(),
    commandRouter: { handleEvent: vi.fn() },
    invalidateSearch: vi.fn(), handleSearchQuery: vi.fn(), searchNext: vi.fn(), searchPrevious: vi.fn(),
    handleSearchHighlight: vi.fn(), sanitize: vi.fn(), toggleDarkMode: vi.fn(),
    openBionicReading: vi.fn(), runOcr: vi.fn(), speak: vi.fn(), stopSpeaking: vi.fn(), burnInPdf: vi.fn(),
    exportImage: vi.fn(),
    scanSensitiveData: vi.fn(), reconcileSensitiveDataContext: vi.fn(),
    getSensitiveDataHighlightsForPage: vi.fn(() => []),
  })),
}));

vi.mock("./services/app/presentationController.js", () => ({
  createPresentationController: vi.fn(() => ({
    showToast: vi.fn(), setError: vi.fn(), showPasswordAttempt: vi.fn(), showPasswordError: vi.fn(),
    dismissPasswordAttempt: vi.fn(),
  })),
}));

vi.mock("./services/platform/events.js", () => ({
  listen: vi.fn(async () => vi.fn()),
}));

vi.mock("./services/backend/ai.js", () => ({
  configureAiEndpoint: vi.fn(async (url) => ({
    policy_version: 1,
    mode: "Loopback",
    scheme: "http",
    canonical_host: "127.0.0.1",
    effective_port: 11434,
    canonical_origin: url || "http://127.0.0.1:11434",
    endpoint_fingerprint: "mock_fp",
    created_at_ms: Date.now(),
  })),
  getAiEndpointStatus: vi.fn(async () => ({
    policy_version: 1,
    mode: "Loopback",
    scheme: "http",
    canonical_host: "127.0.0.1",
    effective_port: 11434,
    canonical_origin: "http://127.0.0.1:11434",
    endpoint_fingerprint: "mock_fp",
    created_at_ms: Date.now(),
  })),
  clearAiEndpointConfig: vi.fn(),
  cancelAiRequest: vi.fn(),
  buildSemanticIndex: vi.fn(),
  semanticSearch: vi.fn(),
  queryDocumentRag: vi.fn(),
  summarizeSelectionOrPage: vi.fn(),
}));

import App from "./App.svelte";
import AppHeader from "./components/AppHeader.svelte";
import AppModalHost from "./components/AppModalHost.svelte";
import SearchHost from "./components/SearchHost.svelte";
import ViewerHudDock from "./components/ViewerHudDock.svelte";
import ViewerShell from "./components/ViewerShell.svelte";
import { createDocumentSessionStore } from "./stores/documentSession.svelte.js";
import { createUiState } from "./stores/uiState.svelte.js";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

const feature = (visibleIds = []) => (id) => ({
  visible: visibleIds.includes(id),
  runnable: visibleIds.includes(id),
  warning: "",
});

function createMemoryStorage(initialValues = {}) {
  const values = new Map(Object.entries(initialValues));
  return {
    getItem: vi.fn((key) => values.get(key) ?? null),
    setItem: vi.fn((key, value) => values.set(key, String(value))),
    removeItem: vi.fn((key) => values.delete(key)),
    clear: vi.fn(() => values.clear()),
  };
}

function renderModalHost(ui) {
  return render(AppModalHost, {
    props: {
      ui,
      featureUi: feature(["local_ai"]),
      sessionId: "session-a",
      contentGeneration: 1,
      displayPath: "/a.pdf",
      onCommandSelect: vi.fn(),
      onSanitize: vi.fn(),
    },
  });
}

describe("7R.6 component smoke behavior", () => {
  it("mounts the App shell with mocked runtime dependencies and runs runtime cleanup on unmount", async () => {
    const view = render(App);
    await waitFor(() => expect(appRuntime.mount).toHaveBeenCalledOnce());
    expect(document.querySelector("main.app-container")).not.toBeNull();

    view.unmount();

    expect(appRuntime.cleanup).toHaveBeenCalledOnce();
  });

  it("opens and closes SearchHost through the typed panel API", async () => {
    const ui = createUiState();
    const view = render(SearchHost, {
      props: {
        ui,
        searchState: { query: "", totalMatches: 0, currentMatchIndex: 0, isSearching: false, error: "" },
        featureUi: feature(["search"]),
        onSearch: vi.fn(), onNext: vi.fn(), onPrevious: vi.fn(),
      },
    });

    expect(view.queryByPlaceholderText("BELGE İÇİNDE ARA...")).toBeNull();
    ui.setPanel("search", true);
    flushSync();
    expect(view.getByPlaceholderText("BELGE İÇİNDE ARA...")).toBeTruthy();

    await fireEvent.click(view.getByTitle("Aramayı Kapat (Esc)"));
    expect(ui.panels.search).toBe(false);
    expect(view.queryByPlaceholderText("BELGE İÇİNDE ARA...")).toBeNull();
  });

  it("routes two typed AI modal identities and never mounts assistant with settings", async () => {
    vi.stubGlobal("localStorage", createMemoryStorage());
    if (!Element.prototype.animate) {
      Element.prototype.animate = vi.fn(() => ({
        cancel: vi.fn(),
        finished: Promise.resolve(),
        onfinish: null,
      }));
    }
    const ui = createUiState();
    const view = renderModalHost(ui);

    ui.openModal("ai_assistant");
    flushSync();
    expect(view.getByText("Yapay Zeka (Ollama)")).toBeTruthy();
    expect(view.queryByText("Yapay Zeka Ayarları (Ollama)")).toBeNull();

    await fireEvent.click(view.getByTitle("Ayarlar"));
    flushSync();
    expect(ui.isModalOpen("ai_settings")).toBe(true);
    await waitFor(() => {
      expect(view.queryByText("Yapay Zeka (Ollama)")).toBeNull();
      expect(view.getByText("Yapay Zeka Ayarları (Ollama)")).toBeTruthy();
    });

    ui.closeModal("ai_assistant");
    flushSync();
    expect(ui.isModalOpen("ai_settings")).toBe(true);
    await fireEvent.click(view.getByRole("button", { name: "İptal" }));
    expect(ui.activeModal).toBeNull();
  });

  it("loads all persisted AI settings when the typed settings modal opens", () => {
    vi.stubGlobal("localStorage", createMemoryStorage({
      ai_settings_v1: JSON.stringify({
        schemaVersion: 1,
        endpointDraft: "http://127.0.0.1:22444",
        llmModel: "qwen3:8b",
        embedModel: "mxbai-embed-large",
      }),
    }));
    const ui = createUiState();
    ui.openModal("ai_settings");

    const view = renderModalHost(ui);

    expect(view.getByLabelText("Ollama Endpoint URL").value).toBe("http://127.0.0.1:22444");
    expect(view.getByLabelText("LLM Modeli (Chat / Özetleme)").value).toBe("qwen3:8b");
    expect(view.getByLabelText("Embedding Modeli (Semantik Arama)").value).toBe("mxbai-embed-large");
  });

  it("persists all edited AI settings, closes settings, and exposes them to the next assistant mount", async () => {
    const storage = createMemoryStorage();
    vi.stubGlobal("localStorage", storage);
    const ui = createUiState();
    ui.openModal("ai_settings");
    const view = renderModalHost(ui);

    await fireEvent.input(view.getByLabelText("Ollama Endpoint URL"), { target: { value: "http://127.0.0.1:33444" } });
    await fireEvent.input(view.getByLabelText("LLM Modeli (Chat / Özetleme)"), { target: { value: "gemma3:12b" } });
    await fireEvent.input(view.getByLabelText("Embedding Modeli (Semantik Arama)"), { target: { value: "snowflake-arctic-embed" } });
    await fireEvent.click(view.getByRole("button", { name: "Kaydet" }));

    expect(storage.setItem).toHaveBeenCalledWith(
      "ai_settings_v1",
      expect.stringContaining('"endpointDraft":"http://127.0.0.1:33444"'),
    );
    expect(ui.activeModal).toBeNull();

    storage.getItem.mockClear();
    ui.openModal("ai_assistant");
    flushSync();

    expect(view.getByText("Yapay Zeka (Ollama)")).toBeTruthy();
    expect(ui.activeModal?.type).toBe("ai_assistant");
  });

  it("closes AI settings without changing persisted values when cancelled", async () => {
    const storage = createMemoryStorage({
      ollamaUrl: "http://127.0.0.1:11434",
      llmModel: "llama3",
      embedModel: "nomic-embed-text",
    });
    vi.stubGlobal("localStorage", storage);
    const ui = createUiState();
    ui.openModal("ai_settings");
    const view = renderModalHost(ui);

    await fireEvent.input(view.getByLabelText("Ollama Endpoint URL"), { target: { value: "http://example.invalid" } });
    await fireEvent.input(view.getByLabelText("LLM Modeli (Chat / Özetleme)"), { target: { value: "unsaved-llm" } });
    await fireEvent.input(view.getByLabelText("Embedding Modeli (Semantik Arama)"), { target: { value: "unsaved-embed" } });
    await fireEvent.click(view.getByRole("button", { name: "İptal" }));

    expect(storage.setItem).not.toHaveBeenCalled();
    expect(storage.getItem("ollamaUrl")).toBe("http://127.0.0.1:11434");
    expect(storage.getItem("llmModel")).toBe("llama3");
    expect(storage.getItem("embedModel")).toBe("nomic-embed-text");
    expect(ui.activeModal).toBeNull();
  });

  it("routes the sidebar toggle through the typed panel API", async () => {
    const ui = { panels: { sidebar: true, a11y: false, prepress: false }, togglePanel: vi.fn(), isModalOpen: vi.fn(() => false), openModal: vi.fn() };
    const view = render(AppHeader, {
      props: {
        isPdf: true, viewer: { activeImageSourcePath: "", fileExt: "pdf" }, ui,
        fileName: "a.pdf", activePdfDisplayPath: "/a.pdf", featureUi: feature([]),
        featureControlLabel: (_id, label) => label, onFeatureAction: vi.fn(), onSelectFile: vi.fn(),
      },
    });

    await fireEvent.click(view.getByTitle("Sol Paneli Aç/Kapat (Cmd + B)"));
    expect(ui.togglePanel).toHaveBeenCalledWith("sidebar");
  });

  it("shows the save control only when the backend session snapshot derives dirty state", async () => {
    const documentSession = createDocumentSessionStore();
    documentSession.commitCandidate({
      id: "session-a", lifecycle: "open", contentGeneration: 1, stateRevision: 1,
      dirty: false, displayPath: "/a.pdf", fileName: "a.pdf",
    });
    const props = {
      isPdf: true,
      viewer: {
        activeImageSourcePath: "", pageOrder: [0], currentPageIndex: 0, scale: 1,
        historyStack: [], forwardStack: [],
        isSinglePageMode: false, isBookMode: false, coverMode: false, isSmartDarkMode: false,
        isSpeaking: false,
      },
      ui: { panels: { search: false, readingRuler: false }, settingsPopover: false, isModalOpen: () => false, openModal: vi.fn(), togglePanel: vi.fn() },
      annotation: { activeTool: "pointer", activeColor: "#fff", strokeColor: "#fff", strokeSize: 2 },
      featureUi: feature(["annotation"]), featureControlLabel: (_id, label) => label,
      colorPalette: ["#fff"], isDirty: documentSession.dirty, isLoadedPdfDocument: true,
      onFeatureAction: vi.fn(), onGoBack: vi.fn(), onGoForward: vi.fn(), onPreviousPage: vi.fn(),
      onNextPage: vi.fn(), onPageInputChange: vi.fn(), onRotateLeft: vi.fn(), onRotateRight: vi.fn(),
      onFlipH: vi.fn(), onFlipV: vi.fn(), onZoomOut: vi.fn(), onResetZoom: vi.fn(), onZoomIn: vi.fn(),
      onToggleSinglePage: vi.fn(), onToggleBookMode: vi.fn(), onToggleDarkMode: vi.fn(), onOpenBionic: vi.fn(),
      onRunOcr: vi.fn(), onSpeak: vi.fn(), onStopSpeaking: vi.fn(), onSave: vi.fn(), onBurnIn: vi.fn(), onExportImage: vi.fn(),
    };
    const view = render(ViewerHudDock, { props });
    expect(view.queryByTitle("Çalışma kopyasını atomik olarak kaydet")).toBeNull();

    documentSession.applySnapshot({ ...documentSession.active, stateRevision: 2, dirty: true });
    await view.rerender({ ...props, isDirty: documentSession.dirty });
    const save = view.getByTitle("Çalışma kopyasını atomik olarak kaydet");
    expect(save).toBeTruthy();
    await fireEvent.click(save);
    expect(props.onSave).toHaveBeenCalledOnce();
  });

  it("selects idle, independent image, and PDF viewer shells from authoritative state", async () => {
    const viewer = {
      activeImageSourcePath: "", visualScale: 1, isBookMode: false, isSplitView: false,
      isSmartDarkMode: false, isZooming: false, isRendering: false, pageOrder: [], currentPageIndex: 0,
      scale: 1, rotation: 0, renderGenerationId: 0,
    };
    const base = {
      isPdf: false, viewer,
      annotation: { activeTool: "pointer", activeColor: "#fff" },
      documentSession: { activeSessionId: null, contentGeneration: 0 },
      searchState: { requestId: 0, matchesByPage: {} }, spreads: [], physicalPageIndex: -1,
      prepress: { heatmap: null, separations: null, channels: {}, boxes: null },
      featureUi: feature([]), errorMsg: "", getActivePrepressPayload: () => null,
      onSelectFile: vi.fn(), onPageIntersect: vi.fn(), onSearchHighlightReady: vi.fn(),
      onSessionSnapshot: vi.fn(), onCanvasMouseDown: vi.fn(), onCanvasMouseMove: vi.fn(), onCanvasMouseUp: vi.fn(),
    };
    const view = render(ViewerShell, { props: base });
    expect(view.getByText("// DOSYA_YÜKLEME_MERKEZİ")).toBeTruthy();

    viewer.activeImageSourcePath = "/image.png";
    await view.rerender({ ...base, viewer: { ...viewer } });
    expect(document.querySelector(".viewer-area canvas")).not.toBeNull();
    expect(view.queryByText("// DOSYA_YÜKLEME_MERKEZİ")).toBeNull();

    await view.rerender({ ...base, isPdf: true, viewer: { ...viewer, activeImageSourcePath: "" } });
    expect(document.querySelector(".viewer-area .pdf-continuous-scroll")).not.toBeNull();
    expect(document.querySelector(".viewer-area canvas")).toBeNull();
  });
});
