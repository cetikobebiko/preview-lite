import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import {
  cancelRenderRequest,
  getPageSizes,
  getPageTextLayout,
  renderPage,
  retryPage,
} from "../services/backend/viewer.js";
import { pageFailureRegistry } from "../services/pageFailureRegistry.js";
import PdfPage from "./PdfPage.svelte";
import {
  addHighlight,
  bindHighlightSession,
  remapHighlightPages,
  resetHighlightsForTests,
} from "../annotationsStore.js";
import { pageMappingAfterDelete } from "../services/app/pageIndexMapping.js";

const renderSessionMock = vi.hoisted(() => ({
  current: {
    sessionId: "test-document",
    contentGeneration: 1,
  },
  snapshot() {
    return this.current;
  },
}));

vi.mock("svelte", async () => {
  const client = await import("../../node_modules/svelte/src/index-client.js");
  return {
    ...client,
    getContext(key) {
      return client.getContext(key) ?? renderSessionMock;
    },
  };
});

vi.mock("../services/backend/viewer.js", () => ({
  cancelRenderRequest: vi.fn(async () => ({ cancelled: true })),
  getPageSizes: vi.fn(async () => null),
  getPageTextLayout: vi.fn(),
  renderPage: vi.fn(),
  retryPage: vi.fn(async () => true),
}));

vi.mock("../pdfState.js", () => ({
  getPageAnnotations: vi.fn(async () => []),
  addAnnotation: vi.fn(async () => undefined),
  deleteAnnotation: vi.fn(async () => undefined),
  getFormFields: vi.fn(async () => []),
  updateFormField: vi.fn(async () => undefined),
  detectScannedPages: vi.fn(async () => []),
  runOcrOnPage: vi.fn(async () => ({ words: [] })),
  getAvailableOcrLanguages: vi.fn(async () => ["eng"]),
}));

vi.mock("../featureRegistry.js", () => ({
  getRuntimeFeatureContext: vi.fn(() => ({})),
  getFeaturePresentation: vi.fn(() => ({ runnable: true })),
}));

const originalImageData = globalThis.ImageData;
const originalIntersectionObserver = globalThis.IntersectionObserver;
const originalResizeObserver = globalThis.ResizeObserver;
const originalGetContext = HTMLCanvasElement.prototype.getContext;

function renderedPage(rotation = 0) {
  const sideways = rotation === 90 || rotation === 270;
  const width = sideways ? 10 : 20;
  const height = sideways ? 20 : 10;
  return {
    width,
    height,
    data: new Uint8Array(width * height * 4).fill(255),
  };
}

function textLayout(text = "word") {
  return [{ x: 0.1, y: 0.2, width: 0.3, height: 0.2, text }];
}

function textRequestCount() {
  return getPageTextLayout.mock.calls.length;
}

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function renderPdfPage(options) {
  renderSessionMock.current = {
    sessionId: options.props.sessionId,
    contentGeneration: options.props.documentGenerationId,
  };
  return render(PdfPage, options);
}

beforeEach(() => {
  pageFailureRegistry.reset();
  renderSessionMock.current = {
    sessionId: "test-document",
    contentGeneration: 1,
  };
  globalThis.ImageData = class ImageDataMock {
    constructor(data, width, height) {
      this.data = data;
      this.width = width;
      this.height = height;
    }
  };

  globalThis.IntersectionObserver = class IntersectionObserverMock {
    observe() {}
    disconnect() {}
  };
  globalThis.ResizeObserver = class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
  };

  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({
    fillStyle: "",
    font: "",
    fillRect: vi.fn(),
    putImageData: vi.fn(),
    measureText: vi.fn((text) => ({ width: Math.max(text.length * 5, 1) })),
  }));

  renderPage.mockImplementation(async (request) => ({
    ...renderedPage(request.rotation),
    pageIndex: request.pageIndex,
    generation: request.contentGeneration,
    requestId: request.requestId,
  }));
  getPageTextLayout.mockResolvedValue(textLayout());
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  globalThis.ImageData = originalImageData;
  globalThis.IntersectionObserver = originalIntersectionObserver;
  globalThis.ResizeObserver = originalResizeObserver;
  HTMLCanvasElement.prototype.getContext = originalGetContext;
});

describe("PdfPage PDFium text layer", () => {
  it("does not let a stale worker failure overwrite a newer successful render", async () => {
    const oldRender = createDeferred();
    const newRender = createDeferred();
    renderPage
      .mockImplementationOnce(() => oldRender.promise)
      .mockImplementationOnce(() => newRender.promise);

    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));

    await rerender({ scale: 1.25 });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
    const newRequest = renderPage.mock.calls[1][0];
    newRender.resolve({
      ...renderedPage(newRequest.rotation),
      pageIndex: newRequest.pageIndex,
      generation: newRequest.contentGeneration,
      requestId: newRequest.requestId,
    });
    await waitFor(() => expect(container.querySelector(".page-error")).toBeNull());

    oldRender.reject(new Error("PDF worker restart budget was exhausted"));
    await Promise.resolve();
    await Promise.resolve();

    expect(container.querySelector(".page-error")).toBeNull();
  });

  it("clears a transient worker failure when a newer request succeeds", async () => {
    const failedRender = createDeferred();
    const recoveredRender = createDeferred();
    renderPage
      .mockImplementationOnce(() => failedRender.promise)
      .mockImplementationOnce(() => recoveredRender.promise);

    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));

    failedRender.reject(new Error("PDF worker restart budget was exhausted"));
    await waitFor(() => {
      expect(container.querySelector(".page-error")?.textContent).toContain(
        "PDF worker restart budget was exhausted",
      );
    });

    await rerender({ scale: 1.25 });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
    expect(container.querySelector(".page-error")).toBeNull();
    const recoveredRequest = renderPage.mock.calls[1][0];
    recoveredRender.resolve({
      ...renderedPage(recoveredRequest.rotation),
      pageIndex: recoveredRequest.pageIndex,
      generation: recoveredRequest.contentGeneration,
      requestId: recoveredRequest.requestId,
    });

    await waitFor(() => expect(container.querySelector(".page-error")).toBeNull());
  });

  it("PDFium sırasını koruyan güvenli span metni ve kontrollü boşluk üretir", async () => {
    getPageTextLayout.mockResolvedValue([
      { x: 0.1, y: 0.1, width: 0.2, height: 0.1, text: "<first&>" },
      { x: 0.4, y: 0.1, width: 0.2, height: 0.1, text: "second" },
    ]);

    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });

    await waitFor(() => {
      expect(container.querySelectorAll(".textLayer span")).toHaveLength(2);
    });

    const layer = container.querySelector(".textLayer");
    const spans = Array.from(layer.querySelectorAll("span"));
    expect(layer.textContent).toBe("<first&> second");
    expect(spans.map((span) => span.textContent)).toEqual(["<first&> ", "second"]);
    expect(layer.querySelector("a")).toBeNull();
    expect(layer.querySelector("first")).toBeNull();
    expect(layer.classList.contains("is-interactive")).toBe(true);
  });

  it("scale, rotasyon, renk ve görünüm nesli değişiminde metni yeniden istemez", async () => {
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 3,
      },
    });

    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
    expect(textRequestCount()).toBe(1);

    await rerender({
      scale: 1.5,
      rotation: 90,
      colorMode: "dark",
      generationId: 1,
    });
    await waitFor(() => {
      expect(container.querySelector(".textLayer span").style.transform).toContain("rotate(90deg)");
    });

    expect(textRequestCount()).toBe(1);

    await rerender({ rotation: 270 });
    await waitFor(() => {
      expect(container.querySelector(".textLayer span").style.transform).toContain("rotate(270deg)");
    });
    expect(textRequestCount()).toBe(1);
  });

  it("belge nesli değiştiğinde aynı dosya ve sayfa için yeni metin isteği yapar", async () => {
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 7,
      },
    });

    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
    expect(textRequestCount()).toBe(1);

    renderSessionMock.current = {
      ...renderSessionMock.current,
      contentGeneration: 8,
    };
    await rerender({ documentGenerationId: 8 });
    await waitFor(() => expect(textRequestCount()).toBe(2));
  });

  it("eski dosyanın geciken metin yanıtını yeni katmana yazmaz", async () => {
    const oldText = createDeferred();
    const newText = createDeferred();

    getPageTextLayout.mockImplementation((sessionId) =>
      sessionId === "old-session" ? oldText.promise : newText.promise
    );

    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "old-session",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });

    await waitFor(() => expect(textRequestCount()).toBe(1));

    renderSessionMock.current = {
      sessionId: "new-session",
      contentGeneration: 2,
    };
    await rerender({ sessionId: "new-session", documentGenerationId: 2 });
    await waitFor(() => expect(textRequestCount()).toBe(2));

    newText.resolve(textLayout("new"));
    await waitFor(() => expect(container.querySelector(".textLayer")?.textContent).toBe("new"));

    oldText.resolve(textLayout("old"));
    await tick();
    expect(container.querySelector(".textLayer").textContent).toBe("new");
  });

  it("görünmezlikte DOM katmanını kaldırır ve tekrar görünürlükte normalize veriyi kullanır", async () => {
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });

    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
    expect(textRequestCount()).toBe(1);

    await rerender({ currentPage: 1 });
    await tick();
    expect(container.querySelector(".textLayer")).toBeNull();

    await rerender({ currentPage: 0 });
    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
    expect(textRequestCount()).toBe(1);
  });

  it("pointer modunda seçimi açar, annotation araçlarında kapatır; redaction katmanı render etmez", async () => {
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });

    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
    expect(container.querySelector(".textLayer").classList.contains("is-interactive")).toBe(true);
    expect(container.querySelector(".redaction-layer")).toBeNull();

    await rerender({ activeTool: "highlight" });
    await tick();
    expect(container.querySelector(".textLayer").classList.contains("is-interactive")).toBe(false);
    expect(container.querySelector(".annotation-layer").classList.contains("drawing")).toBe(true);
    expect(container.querySelector(".redaction-layer")).toBeNull();

    await rerender({ activeTool: "pointer" });
    await tick();
    expect(container.querySelector(".textLayer").classList.contains("is-interactive")).toBe(true);
    expect(container.querySelector(".redaction-layer")).toBeNull();

    const pdfPageSource = readFileSync(resolve(process.cwd(), "src/components/PdfPage.svelte"), "utf8");
    expect(pdfPageSource).not.toContain("export let redactionMode");
    expect(pdfPageSource).not.toContain("export let pageRedactionRects");
  });

  it("mantıksal eşleşmenin çoklu kutularını korur ve aktifi farklı çizer", async () => {
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
        searchRequestId: 12,
        searchMatches: [
          {
            index: 0,
            pageIndex: 0,
            isActive: true,
            text: "active",
            boxes: [
              { x: 0.1, y: 0.2, width: 0.3, height: 0.2 },
              { x: 0.5, y: 0.2, width: 0.2, height: 0.2 },
            ],
          },
          {
            index: 1,
            pageIndex: 0,
            isActive: false,
            text: "passive",
            boxes: [{ x: 0.2, y: 0.6, width: 0.2, height: 0.1 }],
          },
        ],
      },
    });

    await waitFor(() => {
      expect(container.querySelectorAll(".highlight-rect")).toHaveLength(3);
    });

    const activeBoxes = container.querySelectorAll(
      '[data-search-match-index="0"].is-active',
    );
    const passiveBoxes = container.querySelectorAll(
      '[data-search-match-index="1"]:not(.is-active)',
    );
    expect(activeBoxes).toHaveLength(2);
    expect(passiveBoxes).toHaveLength(1);
    expect(activeBoxes[0].style.left).toBe("2px");
    expect(activeBoxes[0].style.top).toBe("2px");
    expect(activeBoxes[0].style.width).toBe("6px");
    expect(activeBoxes[0].style.height).toBe("2px");
    expect(
      getComputedStyle(container.querySelector(".search-highlight-layer")).pointerEvents,
    ).toBe("none");
  });

  it("aktif highlight hazır olayını request, frontend index ve sayfayla bildirir", async () => {
    const ready = vi.fn();
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
        searchRequestId: 20,
        searchMatches: [],
        $$events: { searchHighlightReady: ready },
      },
    });
    await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());

    await rerender({
      searchMatches: [
        {
          index: 3,
          pageIndex: 0,
          isActive: true,
          text: "ready",
          boxes: [{ x: 0.1, y: 0.2, width: 0.3, height: 0.2 }],
        },
      ],
    });

    await waitFor(() => expect(ready).toHaveBeenCalledOnce());
    expect(ready.mock.calls[0][0].detail).toMatchObject({
      searchRequestId: 20,
      matchIndex: 3,
      pageIndex: 0,
    });
    expect(ready.mock.calls[0][0].detail.element).toBe(
      container.querySelector('[data-search-match-index="3"]'),
    );
  });

  it("görünmezlikte highlight DOM'unu kaldırır ve dönüşte mevcut sonucu kullanır", async () => {
    const { container, rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
        searchRequestId: 30,
        searchMatches: [
          {
            index: 0,
            pageIndex: 0,
            isActive: true,
            text: "persist",
            boxes: [{ x: 0.1, y: 0.2, width: 0.3, height: 0.2 }],
          },
        ],
      },
    });

    await waitFor(() => {
      expect(container.querySelector(".search-highlight-layer")).not.toBeNull();
    });

    await rerender({ currentPage: 1 });
    await tick();
    expect(container.querySelector(".search-highlight-layer")).toBeNull();

    await rerender({ currentPage: 0 });
    await waitFor(() => {
      expect(container.querySelectorAll(".highlight-rect")).toHaveLength(1);
    });
  });

  it("renders a separate amber review-highlight-layer only when paintedRenderKey exists and sensitiveDataHighlights are present", async () => {
    const highlights = [
      { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
      { x: 0.5, y: 0.6, width: 0.2, height: 0.1 },
    ];
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
        sensitiveDataHighlights: highlights,
      },
    });

    // Negative verification before PDF render promise completes / paintedRenderKey exists
    expect(container.querySelector(".review-highlight-layer")).toBeNull();

    await waitFor(() => {
      expect(container.querySelector(".review-highlight-layer")).not.toBeNull();
    });

    const layer = container.querySelector(".review-highlight-layer");
    expect(layer.getAttribute("aria-hidden")).toBe("true");
    expect(layer.style.pointerEvents).toBe("none");
    expect(layer.style.transform).toBe("");

    const rects = layer.querySelectorAll(".review-highlight-rect");
    expect(rects).toHaveLength(2);
    expect(rects[0].style.left).toBe("10%");
    expect(rects[0].style.top).toBe("20%");
    expect(rects[0].style.width).toBe("30%");
    expect(rects[0].style.height).toBe("40%");
    expect(rects[0].style.transform).toBe("");

    expect(rects[1].style.left).toBe("50%");
    expect(rects[1].style.top).toBe("60%");

    // No redaction mask classes or mutation callsites
    expect(container.querySelector(".redaction-layer")).toBeNull();
  });

  it("does NOT render review-highlight-layer when sensitiveDataHighlights are non-empty but paintedRenderKey is absent", () => {
    const highlights = [{ x: 0.1, y: 0.2, width: 0.3, height: 0.4 }];
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
        sensitiveDataHighlights: highlights,
      },
    });

    // Synchronously check before render completes
    expect(container.querySelector(".review-highlight-layer")).toBeNull();
  });
});

describe("PdfPage text highlights", () => {
  const rect = { x: 0.1, y: 0.2, width: 0.3, height: 0.05 };
  const pageProps = (sessionId, pageIndex) => ({
    sessionId,
    pageIndex,
    isSinglePageMode: true,
    currentPage: pageIndex,
    documentGenerationId: 1,
  });

  beforeEach(() => resetHighlightsForTests());
  afterEach(() => resetHighlightsForTests());

  it("does not show the previous document's highlight on the same page number", async () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 0, rect);
    const first = renderPdfPage({ props: pageProps("doc-a", 0) });
    await waitFor(() => expect(first.container.querySelectorAll(".highlight-overlay-svg rect")).toHaveLength(1));
    cleanup();

    // Before the reconciler rebinds, a page of another session must not see doc-a highlights.
    const second = renderPdfPage({ props: pageProps("doc-b", 0) });
    await waitFor(() => expect(second.container.querySelector(".overlay-plane")).not.toBeNull());
    await tick();
    expect(second.container.querySelector(".highlight-overlay-svg")).toBeNull();

    bindHighlightSession("doc-b");
    await tick();
    expect(second.container.querySelector(".highlight-overlay-svg")).toBeNull();
  });

  it("shows a highlight on its moved page after a page deletion is remapped", async () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 2, rect);
    const page = renderPdfPage({ props: pageProps("doc-a", 1) });
    await waitFor(() => expect(page.container.querySelector(".overlay-plane")).not.toBeNull());
    await tick();
    expect(page.container.querySelector(".highlight-overlay-svg")).toBeNull();

    remapHighlightPages("doc-a", pageMappingAfterDelete([1]));
    await waitFor(() => expect(page.container.querySelectorAll(".highlight-overlay-svg rect")).toHaveLength(1));
  });
});

describe("PdfPage secure-redaction review selection", () => {
  const pendingRegion = {
    id: "region-1",
    pageIndex: 0,
    normalizedTopLeftRect: { x: 0.1, y: 0.2, width: 0.3, height: 0.25 },
    source: "manual_selection",
  };

  it("does not install review interaction when selection mode is disabled", async () => {
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document", pageIndex: 0, isSinglePageMode: true, currentPage: 0,
        documentGenerationId: 1, secureRedactionEnabled: true,
      },
    });
    await waitFor(() => expect(container.querySelector(".pdf-canvas")).not.toBeNull());
    expect(container.querySelector(".pending-redaction-review-layer")).toBeNull();
  });

  it.each([
    [0, { x: 0.1, y: 0.2, width: 0.3, height: 0.4 }],
    [90, { x: 0.2, y: 0.6, width: 0.4, height: 0.3 }],
    [180, { x: 0.6, y: 0.4, width: 0.3, height: 0.4 }],
    [270, { x: 0.4, y: 0.1, width: 0.4, height: 0.3 }],
  ])("emits strict logical normalized rectangles at rotation %i", async (rotation, expected) => {
    const onRedactionRegionSelected = vi.fn();
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document", pageIndex: 4, isSinglePageMode: true, currentPage: 4,
        documentGenerationId: 1, rotation, secureRedactionEnabled: true,
        secureRedactionSelectionMode: true, onRedactionRegionSelected,
      },
    });
    const layer = await waitFor(() => {
      const element = container.querySelector(".pending-redaction-review-layer");
      expect(element).not.toBeNull();
      return element;
    });
    layer.getBoundingClientRect = () => ({ left: 0, top: 0, right: 200, bottom: 100, width: 200, height: 100, x: 0, y: 0, toJSON() {} });
    await fireEvent.pointerDown(layer, { pointerId: 7, button: 0, clientX: 20, clientY: 20 });
    await fireEvent.pointerMove(layer, { pointerId: 7, clientX: 80, clientY: 60 });
    await fireEvent.pointerUp(layer, { pointerId: 7, button: 0, clientX: 80, clientY: 60 });
    expect(onRedactionRegionSelected).toHaveBeenCalledTimes(1);
    expect(onRedactionRegionSelected).toHaveBeenCalledWith({
      pageIndex: 4,
      normalizedTopLeftRect: expect.objectContaining(expected),
    });
  });

  it("rejects tiny drags and presents pending regions as a dashed non-black review tint", async () => {
    const callback = vi.fn();
    const { container } = renderPdfPage({
      props: {
        sessionId: "test-document", pageIndex: 0, isSinglePageMode: true, currentPage: 0,
        documentGenerationId: 1, secureRedactionEnabled: true, secureRedactionSelectionMode: true,
        pendingRedactionRegions: [pendingRegion], onRedactionRegionSelected: callback,
      },
    });
    const layer = await waitFor(() => container.querySelector(".pending-redaction-review-layer"));
    layer.getBoundingClientRect = () => ({ left: 0, top: 0, right: 200, bottom: 100, width: 200, height: 100, x: 0, y: 0, toJSON() {} });
    await fireEvent.pointerDown(layer, { pointerId: 2, button: 0, clientX: 10, clientY: 10 });
    await fireEvent.pointerUp(layer, { pointerId: 2, button: 0, clientX: 11, clientY: 11 });
    expect(callback).not.toHaveBeenCalled();
    const region = layer.querySelector(".pending-redaction-region");
    expect(region).not.toBeNull();
    expect(region.textContent).toContain("PENDING");
    expect(container.querySelector(".redaction-layer")).toBeNull();
    const source = readFileSync(resolve(process.cwd(), "src/components/PdfPage.svelte"), "utf8");
    expect(source).not.toMatch(/pending-redaction-region[^}]*background:\s*(?:#000|black|rgb\(0,\s*0,\s*0\))/s);
  });
});

describe("PdfPage console output", () => {
  it("renders and drops stale results without console.log", async () => {
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    const staleRender = createDeferred();
    renderPage.mockImplementationOnce(() => staleRender.promise);
    try {
      const { container, rerender } = renderPdfPage({
        props: {
          sessionId: "test-document",
          pageIndex: 0,
          isSinglePageMode: true,
          currentPage: 0,
          documentGenerationId: 1,
        },
      });
      await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));
      const staleRequest = renderPage.mock.calls[0][0];

      await rerender({ scale: 1.25 });
      await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
      await waitFor(() => expect(container.querySelector(".textLayer")).not.toBeNull());
      staleRender.resolve({
        ...renderedPage(staleRequest.rotation),
        pageIndex: staleRequest.pageIndex,
        generation: staleRequest.contentGeneration,
        requestId: staleRequest.requestId,
      });
      await Promise.resolve();
      await Promise.resolve();

      expect(log).not.toHaveBeenCalled();
    } finally {
      log.mockRestore();
    }
  });
});

describe("PdfPage render cancellation", () => {
  it("lets an in-flight render finish during a zoom gesture and cancels once when it settles", async () => {
    const zoomRender = createDeferred();
    renderPage
      .mockImplementationOnce(async (request) => ({
        ...renderedPage(request.rotation),
        pageIndex: request.pageIndex,
        generation: request.contentGeneration,
        requestId: request.requestId,
      }))
      .mockImplementationOnce(() => zoomRender.promise);

    const { rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));
    await tick();

    await rerender({ scale: 1.25 });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));

    // Further gesture steps while the 1.25 render is still in flight.
    for (const scale of [1.5, 1.75, 2]) {
      await rerender({ scale });
    }
    expect(cancelRenderRequest).not.toHaveBeenCalled();

    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(3));
    expect(cancelRenderRequest).toHaveBeenCalledOnce();
    expect(cancelRenderRequest).toHaveBeenCalledWith(
      expect.objectContaining({
        requestId: renderPage.mock.calls[1][0].requestId,
        scope: "main_page",
      }),
      "zoom",
    );
    expect(renderPage.mock.calls[2][0].scale).toBe(
      renderPage.mock.calls[0][0].scale * 2,
    );
    zoomRender.reject(new Error("render_cancelled"));
  });

  it("tags cancellation of a page leaving the render buffer", async () => {
    const pendingRender = createDeferred();
    renderPage.mockImplementationOnce(() => pendingRender.promise);

    const { rerender } = renderPdfPage({
      props: {
        sessionId: "test-document",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));

    await rerender({ currentPage: 1 });
    expect(cancelRenderRequest).toHaveBeenCalledWith(
      expect.objectContaining({
        requestId: renderPage.mock.calls[0][0].requestId,
      }),
      "viewport_exit",
    );
    pendingRender.reject(new Error("render_cancelled"));
  });
});

describe("PdfPage visibility reporting", () => {
  it("reports the newest intersection ratio and zero when hidden or destroyed", async () => {
    const callbacks = [];
    globalThis.IntersectionObserver = class IntersectionObserverMock {
      constructor(callback) { callbacks.push(callback); }
      observe() {}
      disconnect() {}
    };
    const onIntersect = vi.fn();
    const { unmount } = renderPdfPage({
      props: { sessionId: "test-document", pageIndex: 3, documentGenerationId: 1, onIntersect },
    });
    const visibility = callbacks[0];
    visibility([{ isIntersecting: true, intersectionRatio: 0.2 }, { isIntersecting: true, intersectionRatio: 0.6 }]);
    expect(onIntersect).toHaveBeenLastCalledWith(3, 0.6);
    visibility([{ isIntersecting: false, intersectionRatio: 0 }]);
    expect(onIntersect).toHaveBeenLastCalledWith(3, 0);
    onIntersect.mockClear();
    unmount();
    expect(onIntersect).toHaveBeenCalledWith(3, 0);
  });
});

describe("PdfPage placeholder geometry and single-page mode", () => {
  const LETTER = { widthPt: 612, heightPt: 792 };
  const A4_LANDSCAPE = { widthPt: 841.89, heightPt: 595.276 };

  function pageContainer(container) {
    return container.querySelector(".pdf-page-container");
  }

  it("sizes unrendered pages from the worker page size of each page", async () => {
    getPageSizes.mockResolvedValueOnce([LETTER, A4_LANDSCAPE]);
    const { container: first } = renderPdfPage({
      props: { sessionId: "mixed-sizes", pageIndex: 0, documentGenerationId: 1 },
    });
    const { container: second } = renderPdfPage({
      props: { sessionId: "mixed-sizes", pageIndex: 1, documentGenerationId: 1, scale: 1.3 },
    });

    await waitFor(() => expect(pageContainer(first).style.height).toBe("792px"));
    // trunc(841.89 * 1.3) = 1094 px wide; round(595.276 * 1094 / 841.89) = 774 px high.
    await waitFor(() => expect(pageContainer(second).style.height).toBe("774px"));
    expect(getPageSizes).toHaveBeenCalledTimes(1);
    expect(getPageSizes).toHaveBeenCalledWith("mixed-sizes", 1);
  });

  it("asks for page sizes again when the content generation changes", async () => {
    getPageSizes
      .mockResolvedValueOnce([LETTER])
      .mockResolvedValueOnce([A4_LANDSCAPE]);
    const { container, rerender } = renderPdfPage({
      props: { sessionId: "generation-sizes", pageIndex: 0, documentGenerationId: 1 },
    });
    await waitFor(() => expect(pageContainer(container).style.height).toBe("792px"));

    await rerender({ documentGenerationId: 2 });
    await waitFor(() => expect(pageContainer(container).style.height).toBe("595px"));
    expect(getPageSizes).toHaveBeenLastCalledWith("generation-sizes", 2);
  });

  it("reports no layout shift when the placeholder matched the rendered page", async () => {
    getPageSizes.mockResolvedValueOnce([LETTER]);
    renderPage.mockImplementationOnce(async (request) => ({
      width: 612,
      height: 792,
      data: new Uint8Array(612 * 792 * 4).fill(255),
      pageIndex: request.pageIndex,
      generation: request.contentGeneration,
      requestId: request.requestId,
    }));
    const { container } = renderPdfPage({
      props: {
        sessionId: "delta-matched",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(pageContainer(container).style.height).toBe("792px"));
    await waitFor(() =>
      expect(pageContainer(container).dataset.placeholderHeightDelta).toBe("0"),
    );
  });

  it("measures the layout shift when the placeholder had to guess", async () => {
    const { container } = renderPdfPage({
      props: {
        sessionId: "delta-guessed",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    // No worker size: A4 fallback (841 px) before the 20x10 render arrives.
    await waitFor(() =>
      expect(pageContainer(container).dataset.placeholderHeightDelta).toBe(String(10 - 841)),
    );
    expect(pageContainer(container).style.height).toBe("10px");
  });

  it("forgets the measured fallback size when another document is shown", async () => {
    const { container: rendered } = renderPdfPage({
      props: {
        sessionId: "fallback-a",
        pageIndex: 0,
        isSinglePageMode: true,
        currentPage: 0,
        documentGenerationId: 1,
      },
    });
    await waitFor(() => expect(pageContainer(rendered).style.height).toBe("10px"));

    // Same document: the measured 20x10 page is reused for unrendered pages.
    const { container: sameDocument } = renderPdfPage({
      props: { sessionId: "fallback-a", pageIndex: 1, documentGenerationId: 1 },
    });
    expect(pageContainer(sameDocument).style.height).toBe("10px");

    // Other document: back to the A4 default instead of the previous document's ratio.
    const { container: otherDocument } = renderPdfPage({
      props: { sessionId: "fallback-b", pageIndex: 1, documentGenerationId: 1 },
    });
    expect(pageContainer(otherDocument).style.height).toBe("841px");
  });

  it("renders only the active page in single-page mode and leaves the others empty", async () => {
    getPageSizes.mockResolvedValueOnce([LETTER, LETTER, A4_LANDSCAPE]);
    const pages = [0, 1, 2].map((pageIndex) =>
      renderPdfPage({
        props: {
          sessionId: "single-page",
          pageIndex,
          isSinglePageMode: true,
          currentPage: 1,
          documentGenerationId: 1,
        },
      }).container,
    );

    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(1));
    expect(renderPage.mock.calls[0][0].pageIndex).toBe(1);
    for (const index of [0, 2]) {
      const shell = pageContainer(pages[index]);
      expect(shell.children).toHaveLength(0);
      expect(shell.textContent).not.toContain("Yükleniyor");
      expect(shell.getAttribute("aria-hidden")).toBe("true");
    }
    expect(pageContainer(pages[1]).querySelector("canvas")).not.toBeNull();
    // Inactive pages keep their real height so page navigation can still scroll to them.
    await waitFor(() => expect(pageContainer(pages[2]).style.height).toBe("595px"));
  });

  it("keeps the loading placeholder for off-screen pages in continuous mode", () => {
    const { container } = renderPdfPage({
      props: { sessionId: "continuous", pageIndex: 4, documentGenerationId: 1 },
    });
    expect(pageContainer(container).textContent).toContain("Sayfa 5 Yükleniyor");
    expect(pageContainer(container).hasAttribute("aria-hidden")).toBe(false);
  });
});

function workerFailure(code, message = "worker failure") {
  return Object.assign(new Error(message), { code });
}

function resolvedRender(request) {
  return {
    ...renderedPage(request.rotation),
    pageIndex: request.pageIndex,
    generation: request.contentGeneration,
    requestId: request.requestId,
  };
}

describe("PdfPage worker-safety failures", () => {
  const singlePage = {
    sessionId: "test-document",
    pageIndex: 0,
    isSinglePageMode: true,
    currentPage: 0,
    documentGenerationId: 1,
  };

  it("does not request a timed-out page again on zoom or when it comes back into view", async () => {
    renderPage.mockRejectedValueOnce(workerFailure("PDFIUM_WORKER_TIMEOUT"));
    const { container, rerender } = renderPdfPage({ props: singlePage });
    await waitFor(() => {
      expect(container.querySelector(".page-error")?.textContent).toContain(
        "Sayfa 1 işlenirken zaman aşımı oluştu.",
      );
    });

    await rerender({ scale: 1.5 });
    await rerender({ currentPage: 1 });
    await rerender({ currentPage: 0 });
    await tick();

    expect(renderPage).toHaveBeenCalledTimes(1);
    expect(container.querySelector(".page-error")?.textContent).toContain(
      "zaman aşımı",
    );
    expect(container.querySelector(".page-retry")).not.toBeNull();
  });

  it("retries a failed page with exactly one request and recovers on success", async () => {
    renderPage.mockRejectedValueOnce(workerFailure("PDFIUM_PAGE_QUARANTINED"));
    const { container } = renderPdfPage({ props: singlePage });
    const retry = await waitFor(() => {
      const button = container.querySelector(".page-retry");
      expect(button).not.toBeNull();
      return button;
    });
    expect(container.querySelector(".page-error")?.textContent).toContain(
      "tekrarlanan zaman aşımları",
    );

    renderPage.mockImplementationOnce(async (request) => resolvedRender(request));
    await fireEvent.click(retry);

    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
    expect(retryPage).toHaveBeenCalledWith("test-document", 1, 0);
    await waitFor(() => expect(container.querySelector(".page-error")).toBeNull());
    await tick();
    expect(renderPage).toHaveBeenCalledTimes(2);
  });

  it("stops after a failed retry instead of retrying automatically", async () => {
    renderPage
      .mockRejectedValueOnce(workerFailure("PDFIUM_WORKER_TIMEOUT"))
      .mockRejectedValueOnce(workerFailure("PDFIUM_PAGE_QUARANTINED"));
    const { container } = renderPdfPage({ props: singlePage });
    const retry = await waitFor(() => {
      const button = container.querySelector(".page-retry");
      expect(button).not.toBeNull();
      return button;
    });

    await fireEvent.click(retry);
    await waitFor(() => {
      expect(container.querySelector(".page-error")?.textContent).toContain(
        "askıya alındı",
      );
    });
    await tick();
    expect(renderPage).toHaveBeenCalledTimes(2);
  });

  it("blocks every page of a quarantined document and offers no page retry", async () => {
    renderPage.mockRejectedValueOnce(workerFailure("PDFIUM_DOCUMENT_QUARANTINED"));
    const { container } = renderPdfPage({ props: singlePage });
    await waitFor(() => {
      expect(container.querySelector(".page-error")?.textContent).toContain(
        "Belge PDF motorunu tekrar tekrar durdurduğu için askıya alındı.",
      );
    });
    expect(container.querySelector(".page-retry")).toBeNull();

    const other = render(PdfPage, {
      props: { ...singlePage, pageIndex: 1, currentPage: 1 },
    });
    await tick();
    expect(renderPage).toHaveBeenCalledTimes(1);
    expect(other.container.querySelector(".page-error")?.textContent).toContain(
      "askıya alındı",
    );
  });

  it("does not request a timed-out text layout again after the page comes back", async () => {
    getPageTextLayout.mockRejectedValueOnce(workerFailure("PDFIUM_WORKER_TIMEOUT"));
    const { rerender } = renderPdfPage({ props: singlePage });
    await waitFor(() => expect(textRequestCount()).toBe(1));
    await tick();

    await rerender({ currentPage: 1 });
    await rerender({ currentPage: 0 });
    await tick();

    expect(textRequestCount()).toBe(1);
  });

  it("requests the page again for a new content generation", async () => {
    renderPage.mockRejectedValueOnce(workerFailure("PDFIUM_WORKER_TIMEOUT"));
    const { container, rerender } = renderPdfPage({ props: singlePage });
    await waitFor(() => expect(container.querySelector(".page-retry")).not.toBeNull());

    renderSessionMock.current = { sessionId: "test-document", contentGeneration: 2 };
    await rerender({ documentGenerationId: 2 });

    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
    expect(renderPage.mock.calls[1][0].contentGeneration).toBe(2);
  });

  it("keeps ordinary failures retryable by a later request", async () => {
    renderPage.mockRejectedValueOnce(new Error("PDF worker restart budget was exhausted"));
    const { container, rerender } = renderPdfPage({ props: singlePage });
    await waitFor(() => expect(container.querySelector(".page-error")).not.toBeNull());
    expect(container.querySelector(".page-retry")).toBeNull();

    await rerender({ scale: 1.5 });
    await waitFor(() => expect(renderPage).toHaveBeenCalledTimes(2));
  });
});
