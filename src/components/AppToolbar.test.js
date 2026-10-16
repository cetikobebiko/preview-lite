import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import AppToolbar from "./AppToolbar.svelte";
import MarkupToolbar from "./MarkupToolbar.svelte";
import { createUiState } from "../stores/uiState.svelte.js";

afterEach(() => cleanup());

const feature = (visibleIds = []) => (id) => ({
  visible: visibleIds.includes(id),
  runnable: visibleIds.includes(id),
  warning: "",
  badge: "",
});

function props(overrides = {}) {
  return {
    isPdf: true,
    viewer: {
      activeImageSourcePath: "", fileExt: "pdf", pageOrder: [0, 1, 2], currentPageIndex: 1, scale: 1, visualScale: 1.5,
      historyStack: [], forwardStack: [], isSinglePageMode: false, isBookMode: false, coverMode: false,
      isSmartDarkMode: false, isSpeaking: false, flipH: false, flipV: false,
    },
    ui: createUiState(),
    annotation: { activeTool: "pointer", activeColor: "#ffcc00", strokeColor: "#ffffff", strokeSize: 2 },
    fileName: "rapor.pdf", activePdfDisplayPath: "/belgeler/rapor.pdf", featureUi: feature([]), isDirty: false,
    onFeatureAction: vi.fn((_id, run) => run()), onSelectFile: vi.fn(), onGoBack: vi.fn(), onGoForward: vi.fn(),
    onPreviousPage: vi.fn(), onNextPage: vi.fn(), onPageInputChange: vi.fn(), onRotateLeft: vi.fn(), onRotateRight: vi.fn(),
    onFlipH: vi.fn(), onFlipV: vi.fn(), onZoomOut: vi.fn(), onResetZoom: vi.fn(), onZoomIn: vi.fn(),
    onToggleSinglePage: vi.fn(), onToggleBookMode: vi.fn(), onToggleDarkMode: vi.fn(), onOpenBionic: vi.fn(),
    onRunOcr: vi.fn(), onSpeak: vi.fn(), onStopSpeaking: vi.fn(), onSave: vi.fn(), onExportImage: vi.fn(),
    ...overrides,
  };
}

describe("AppToolbar", () => {
  it("shows the document title, page position and zoom for a PDF", async () => {
    const p = props();
    const view = render(AppToolbar, { props: p });

    expect(view.getByText("rapor.pdf").getAttribute("title")).toBe("/belgeler/rapor.pdf");
    expect(view.getByRole("spinbutton", { name: "Sayfa numarası" }).value).toBe("2");
    await fireEvent.click(view.getByRole("button", { name: "Yakınlaştırmayı sıfırla (150%)" }));
    expect(p.onResetZoom).toHaveBeenCalledOnce();
    await fireEvent.click(view.getByRole("button", { name: "Sonraki sayfa" }));
    expect(p.onNextPage).toHaveBeenCalledOnce();
    expect(view.queryByRole("button", { name: "Sola döndür" })).toBeNull();
  });

  it("routes search through the feature guard", async () => {
    const p = props({ featureUi: feature(["search"]) });
    const view = render(AppToolbar, { props: p });

    await fireEvent.click(view.getByRole("button", { name: "Ara" }));
    expect(p.onFeatureAction).toHaveBeenCalledWith("search", expect.any(Function));
    expect(p.ui.panels.search).toBe(true);
  });

  it("disarms the drawing tool when the markup row is hidden", async () => {
    const p = props({ featureUi: feature(["annotation"]) });
    const view = render(AppToolbar, { props: p });
    const markup = view.getByRole("button", { name: "İşaretleme" });

    await fireEvent.click(markup);
    expect(p.ui.panels.markup).toBe(true);
    p.annotation.activeTool = "highlight";
    await fireEvent.click(markup);
    expect(p.ui.panels.markup).toBe(false);
    expect(p.annotation.activeTool).toBe("pointer");
  });

  it("offers image transforms instead of page navigation for images", () => {
    const p = props({ isPdf: false, viewer: { ...props().viewer, activeImageSourcePath: "/a.png", fileExt: "png" } });
    const view = render(AppToolbar, { props: p });

    expect(view.getByText("PNG görseli")).toBeTruthy();
    expect(view.getByRole("button", { name: "Sola döndür" })).toBeTruthy();
    expect(view.getByRole("button", { name: "Görseli dönüştür" })).toBeTruthy();
    expect(view.queryByRole("button", { name: "Kenar çubuğu" })).toBeNull();
    expect(view.queryByRole("button", { name: "Sonraki sayfa" })).toBeNull();
  });

  it("offers opening a file when nothing is loaded", async () => {
    const p = props({ isPdf: false });
    const view = render(AppToolbar, { props: p });

    await fireEvent.click(view.getByRole("button", { name: "Aç" }));
    expect(p.onSelectFile).toHaveBeenCalledOnce();
    expect(view.queryByRole("button", { name: "Dışa aktar" })).toBeNull();
  });
});

describe("MarkupToolbar", () => {
  it("selects annotation tools and keeps one tool armed", async () => {
    const annotation = { activeTool: "pointer", activeColor: "#ffcc00", strokeColor: "#ffffff", strokeSize: 2 };
    const view = render(MarkupToolbar, {
      props: { annotation, ui: createUiState(), featureUi: feature(["annotation"]), colorPalette: ["#ffffff"] },
    });

    await fireEvent.click(view.getByRole("radio", { name: "Vurgula" }));
    expect(annotation.activeTool).toBe("highlight");
    await fireEvent.click(view.getByRole("radio", { name: "Vurgula" }));
    expect(annotation.activeTool).toBe("highlight");
  });

  it("disables annotation tools whose feature cannot run", () => {
    const featureUi = (id) => ({ visible: id === "annotation", runnable: false, warning: "Kapalı", badge: "EXPERIMENTAL" });
    const view = render(MarkupToolbar, {
      props: { annotation: { activeTool: "pointer", activeColor: "#000000", strokeColor: "#000000", strokeSize: 2 }, ui: createUiState(), featureUi, colorPalette: [] },
    });

    expect(view.getByRole("radio", { name: "Vurgula" }).disabled).toBe(true);
    expect(view.getByRole("radio", { name: "Seçim" }).disabled).toBe(false);
    expect(view.getByText("Deneysel")).toBeTruthy();
  });
});
