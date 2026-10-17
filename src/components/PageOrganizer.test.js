import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import { rotatePage } from "../pdfState.js";
import { renderThumbnail } from "../services/backend/viewer.js";
import { paintBufferToCanvas } from "../utils/canvasUtils.js";
import { pageFailureRegistry } from "../services/pageFailureRegistry.js";
import PageOrganizer from "./PageOrganizer.svelte";
import { SignatureLossDeclinedError } from "../services/signatureLossGuard.js";

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
  const { RENDER_SESSION_CONTEXT } = await import("../services/renderSession.js");
  return {
    ...client,
    // Only the render session is stubbed; other contexts (bits-ui's own) stay real.
    getContext(key) {
      const value = client.getContext(key);
      return value === undefined && key === RENDER_SESSION_CONTEXT ? renderSessionMock : value;
    },
  };
});

vi.mock("../services/backend/viewer.js", () => ({
  cancelRenderRequest: vi.fn(async () => ({ cancelled: true })),
  renderThumbnail: vi.fn(),
}));

vi.mock("../pdfState.js", () => ({
  rotatePage: vi.fn(async () => undefined),
  deletePages: vi.fn(async () => undefined),
  reorderPages: vi.fn(async () => undefined),
}));

vi.mock("../utils/canvasUtils.js", () => ({
  paintBufferToCanvas: vi.fn((canvas, rendered) => {
    canvas.width = rendered.width;
    canvas.height = rendered.height;
    return { width: rendered.width, height: rendered.height };
  }),
}));

function renderedThumbnail(marker = 255, request = {}) {
  return {
    width: 2,
    height: 1,
    data: new Uint8Array(8).fill(marker),
    pageIndex: request.pageIndex ?? 0,
    generation: request.contentGeneration ?? 1,
    requestId: request.requestId ?? "request",
  };
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

async function flushAsyncWork() {
  await Promise.resolve();
  await tick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function renderOrganizer(options) {
  renderSessionMock.current = {
    sessionId: options.props.sessionId,
    contentGeneration: options.props.contentGeneration ?? 1,
  };
  return render(PageOrganizer, options);
}

const originalAlert = globalThis.alert;
const originalConfirm = globalThis.confirm;
const originalAnimate = Element.prototype.animate;

beforeEach(() => {
  pageFailureRegistry.reset();
  globalThis.alert = vi.fn();
  globalThis.confirm = vi.fn(() => true);
  Element.prototype.animate = vi.fn(() => ({
    cancel: vi.fn(),
    finished: Promise.resolve(),
  }));
  renderSessionMock.current = {
    sessionId: "test-document",
    contentGeneration: 1,
  };
  renderThumbnail.mockImplementation(async (request) =>
    renderedThumbnail(255, request)
  );
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  globalThis.alert = originalAlert;
  globalThis.confirm = originalConfirm;
  Element.prototype.animate = originalAnimate;
});

describe("PageOrganizer PDFium thumbnails", () => {
  it("render_thumbnail IPC sözleşmesini doğru argümanlarla kullanır", async () => {
    renderOrganizer({
      props: {
        sessionId: "test-document",
        contentGeneration: 1,
        pageCount: 1,
      },
    });

    await waitFor(() => {
      expect(renderThumbnail).toHaveBeenCalledWith(expect.objectContaining({
        sessionId: "test-document",
        pageIndex: 0,
        targetWidth: 180,
        contentGeneration: 1,
        scope: "organizer_thumbnail",
        requestId: expect.any(String),
      }));
    });
    await waitFor(() => expect(paintBufferToCanvas).toHaveBeenCalledOnce());
  });

  it("bozuk RGBA payload'ını boyamaz ve hata placeholder'ını korur", async () => {
    renderThumbnail.mockResolvedValue({
      width: 2,
      height: 2,
      data: new Uint8Array(3),
    });

    const { getByText, getByLabelText } = renderOrganizer({
      props: {
        sessionId: "broken-session",
        contentGeneration: 1,
        pageCount: 1,
      },
    });

    await waitFor(() => expect(getByText("Önizleme yüklenemedi")).toBeTruthy());
    expect(paintBufferToCanvas).not.toHaveBeenCalled();
    expect(getByLabelText("Sayfa 1 minyatürü").width).toBe(0);
    expect(getByLabelText("Sayfa 1 minyatürü").height).toBe(0);
  });

  it("aynı session ve sayfada döndürme öncesi geciken sonucu eler", async () => {
    const beforeRotation = createDeferred();
    const afterRotation = createDeferred();
    let renderCall = 0;

    renderThumbnail.mockImplementation(() => {
      renderCall += 1;
      return renderCall === 1 ? beforeRotation.promise : afterRotation.promise;
    });

    const { getByTitle } = renderOrganizer({
      props: {
        sessionId: "session-rotate",
        contentGeneration: 4,
        pageCount: 1,
      },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(1));

    rotatePage.mockResolvedValueOnce({ contentGeneration: 5 });
    renderSessionMock.current = {
      sessionId: "session-rotate",
      contentGeneration: 5,
    };
    await fireEvent.click(getByTitle("Sağa Döndür"));
    await waitFor(() =>
      expect(rotatePage).toHaveBeenCalledWith("session-rotate", 4, 0, 90),
    );
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));

    const newPayload = renderedThumbnail(22, renderThumbnail.mock.calls[1][0]);
    afterRotation.resolve(newPayload);
    await waitFor(() => {
      expect(paintBufferToCanvas).toHaveBeenCalledOnce();
      expect(paintBufferToCanvas.mock.calls[0][1]).toBe(newPayload);
    });

    const oldPayload = renderedThumbnail(11, renderThumbnail.mock.calls[0][0]);
    beforeRotation.resolve(oldPayload);
    await flushAsyncWork();
    expect(paintBufferToCanvas).toHaveBeenCalledOnce();
  });

  it("imza uyarısı reddedilen döndürmede hata göstermez ve minyatürü geri getirir", async () => {
    renderThumbnail.mockImplementation(async (request) => renderedThumbnail(7, request));
    const { getByTitle } = renderOrganizer({
      props: { sessionId: "session-signed", contentGeneration: 2, pageCount: 1 },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(1));

    rotatePage.mockRejectedValueOnce(new SignatureLossDeclinedError());
    await fireEvent.click(getByTitle("Sağa Döndür"));
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));
    expect(globalThis.alert).not.toHaveBeenCalled();
  });

  it("kaynak değişince önceki kaynağın geciken sonucunu uygulamaz", async () => {
    const oldSource = createDeferred();
    renderThumbnail
      .mockImplementationOnce(() => oldSource.promise)
      .mockImplementationOnce(async (request) => renderedThumbnail(33, request));

    const { rerender } = renderOrganizer({
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageCount: 1,
      },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(1));

    renderSessionMock.current = {
      sessionId: "new-session",
      contentGeneration: 2,
    };
    await rerender({ sessionId: "new-session", contentGeneration: 2 });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));
    const newPayload = await renderThumbnail.mock.results[1].value;
    await waitFor(() => {
      expect(paintBufferToCanvas).toHaveBeenCalledOnce();
      expect(paintBufferToCanvas.mock.calls[0][1]).toBe(newPayload);
    });

    oldSource.resolve(renderedThumbnail(44, renderThumbnail.mock.calls[0][0]));
    await flushAsyncWork();
    expect(paintBufferToCanvas).toHaveBeenCalledOnce();
  });

  it("aynı session içinde generation değişince yeniden yükler ve eski canvas sonucunu uygulamaz", async () => {
    const oldGeneration = createDeferred();
    renderThumbnail
      .mockImplementationOnce(() => oldGeneration.promise)
      .mockImplementationOnce(async (request) => renderedThumbnail(66, request));

    const { rerender } = renderOrganizer({
      props: {
        sessionId: "same-session",
        contentGeneration: 1,
        pageCount: 1,
      },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(1));

    renderSessionMock.current = {
      sessionId: "same-session",
      contentGeneration: 2,
    };
    await rerender({
      sessionId: "same-session",
      contentGeneration: 2,
      pageCount: 1,
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));
    const currentPayload = await renderThumbnail.mock.results[1].value;
    await waitFor(() => {
      expect(paintBufferToCanvas).toHaveBeenCalledOnce();
      expect(paintBufferToCanvas.mock.calls[0][1]).toBe(currentPayload);
    });

    oldGeneration.resolve(
      renderedThumbnail(77, renderThumbnail.mock.calls[0][0]),
    );
    await flushAsyncWork();
    expect(paintBufferToCanvas).toHaveBeenCalledOnce();
  });

  it("unmount sonrası geciken sonucu uygulamaz ve canvas backing store'u boşaltır", async () => {
    const pending = createDeferred();
    renderThumbnail.mockReturnValue(pending.promise);

    const { getByLabelText, unmount } = renderOrganizer({
      props: {
        sessionId: "unmount-session",
        contentGeneration: 1,
        pageCount: 1,
      },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(1));

    const canvas = getByLabelText("Sayfa 1 minyatürü");
    unmount();
    expect(canvas.width).toBe(0);
    expect(canvas.height).toBe(0);

    pending.resolve(renderedThumbnail(55, renderThumbnail.mock.calls[0][0]));
    await flushAsyncWork();
    expect(paintBufferToCanvas).not.toHaveBeenCalled();
  });

  it("IPC hatasında kartı kaldırmak yerine hata placeholder'ı gösterir", async () => {
    renderThumbnail.mockRejectedValue(new Error("thumbnail failed"));

    const { getByText } = renderOrganizer({
      props: {
        sessionId: "error-session",
        contentGeneration: 1,
        pageCount: 1,
      },
    });

    await waitFor(() => expect(getByText("Önizleme yüklenemedi")).toBeTruthy());
    expect(paintBufferToCanvas).not.toHaveBeenCalled();
  });
});

describe("PageOrganizer worker-safety failures", () => {
  it("skips a timed-out page on reload and keeps rendering the others", async () => {
    renderThumbnail.mockImplementation(async (request) => {
      if (request.pageIndex === 1) {
        throw Object.assign(new Error("timed out"), { code: "PDFIUM_WORKER_TIMEOUT" });
      }
      return renderedThumbnail(255, request);
    });
    const props = { sessionId: "test-document", contentGeneration: 1, pageCount: 3 };
    const first = renderOrganizer({ props });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(3));
    await flushAsyncWork();
    first.unmount();

    const { container } = renderOrganizer({ props });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(5));
    await flushAsyncWork();
    expect(renderThumbnail.mock.calls.slice(3).map(([request]) => request.pageIndex))
      .toEqual([0, 2]);
    const failed = document.querySelectorAll(".thumbnail-placeholder.error");
    expect(failed).toHaveLength(1);
    expect(failed[0].getAttribute("title")).toBe("Sayfa 2 işlenirken zaman aşımı oluştu.");
  });

  it("stops requesting pages once the document is quarantined", async () => {
    renderThumbnail.mockImplementation(async () => {
      throw Object.assign(new Error("quarantined"), { code: "PDFIUM_DOCUMENT_QUARANTINED" });
    });
    renderOrganizer({
      props: { sessionId: "test-document", contentGeneration: 1, pageCount: 4 },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    await flushAsyncWork();
    await flushAsyncWork();
    expect(renderThumbnail).toHaveBeenCalledOnce();
  });
});
