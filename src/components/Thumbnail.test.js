import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import {
  cancelRenderRequest,
  renderThumbnail,
} from "../services/backend/viewer.js";
import { paintBufferToCanvas } from "../utils/canvasUtils.js";
import { pageFailureRegistry } from "../services/pageFailureRegistry.js";
import Thumbnail from "./Thumbnail.svelte";

const renderSessionMock = vi.hoisted(() => ({
  snapshot: () => ({
    sessionId: "document-1",
    contentGeneration: 4,
  }),
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
  renderThumbnail: vi.fn(),
}));

vi.mock("../utils/canvasUtils.js", () => ({
  paintBufferToCanvas: vi.fn(),
}));

const originalIntersectionObserver = globalThis.IntersectionObserver;
const originalResizeObserver = globalThis.ResizeObserver;
const originalRect = HTMLElement.prototype.getBoundingClientRect;

beforeEach(() => {
  pageFailureRegistry.reset();
  globalThis.IntersectionObserver = class IntersectionObserverMock {
    constructor(callback) {
      this.callback = callback;
    }
    observe(element) {
      this.callback([{ isIntersecting: true, target: element }]);
    }
    disconnect() {}
  };
  globalThis.ResizeObserver = class ResizeObserverMock {
    observe() {}
    disconnect() {}
  };
  HTMLElement.prototype.getBoundingClientRect = vi.fn(() => ({
    width: 123,
    height: 174,
    top: 0,
    left: 0,
    right: 123,
    bottom: 174,
  }));
  renderThumbnail.mockImplementation(async (request) => ({
    width: 2,
    height: 1,
    data: new Uint8Array(8),
    pageIndex: request.pageIndex,
    generation: request.contentGeneration,
    requestId: request.requestId,
  }));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  globalThis.IntersectionObserver = originalIntersectionObserver;
  globalThis.ResizeObserver = originalResizeObserver;
  HTMLElement.prototype.getBoundingClientRect = originalRect;
});

describe("sidebar thumbnail RGBA pipeline", () => {
  it("preserves measured dynamic pixel width and paints validated RGBA", async () => {
    render(Thumbnail, {
      props: { sessionId: "document-1", contentGeneration: 4, pageIndex: 2 },
    });

    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    expect(renderThumbnail).toHaveBeenCalledWith(expect.objectContaining({
      sessionId: "document-1",
      pageIndex: 2,
      targetWidth: 123,
      contentGeneration: 4,
      scope: "sidebar_thumbnail",
      requestId: expect.any(String),
    }));
    await waitFor(() => expect(paintBufferToCanvas).toHaveBeenCalledOnce());
  });

  it("cancels its exact request on unmount", async () => {
    renderThumbnail.mockReturnValue(new Promise(() => {}));
    const { unmount } = render(Thumbnail, {
      props: { sessionId: "document-1", contentGeneration: 4, pageIndex: 1 },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    const request = renderThumbnail.mock.calls[0][0];

    unmount();

    await waitFor(() =>
      expect(cancelRenderRequest).toHaveBeenCalledWith({
        sessionId: request.sessionId,
        pageIndex: request.pageIndex,
        contentGeneration: request.contentGeneration,
        requestId: request.requestId,
        scope: request.scope,
      }, "destroy")
    );
    expect(paintBufferToCanvas).not.toHaveBeenCalled();
  });

  it("does not paint malformed RGBA", async () => {
    renderThumbnail.mockImplementation(async (request) => ({
      width: 2,
      height: 2,
      data: new Uint8Array(3),
      pageIndex: request.pageIndex,
    generation: request.contentGeneration,
      requestId: request.requestId,
    }));
    render(Thumbnail, {
      props: { sessionId: "document-1", contentGeneration: 4, pageIndex: 0 },
    });

    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    await Promise.resolve();
    expect(paintBufferToCanvas).not.toHaveBeenCalled();
  });
});

describe("sidebar thumbnail worker-safety failures", () => {
  const props = { sessionId: "document-1", contentGeneration: 4, pageIndex: 3 };
  const timeout = () =>
    Object.assign(new Error("timed out"), { code: "PDFIUM_WORKER_TIMEOUT" });

  it("does not request a timed-out page again when it is shown again", async () => {
    renderThumbnail.mockRejectedValueOnce(timeout());
    const first = render(Thumbnail, { props });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(first.container.querySelector(".placeholder.failed")).not.toBeNull()
    );
    first.unmount();

    const second = render(Thumbnail, { props });
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(renderThumbnail).toHaveBeenCalledOnce();
    expect(second.container.querySelector(".placeholder")?.getAttribute("title")).toBe(
      "Sayfa 4 işlenirken zaman aşımı oluştu.",
    );
  });

  it("loads again once a successful retry clears the page", async () => {
    renderThumbnail.mockRejectedValueOnce(timeout());
    render(Thumbnail, { props });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(
        pageFailureRegistry.failure({ ...props, surface: "thumbnail" }),
      ).toBe("PDFIUM_WORKER_TIMEOUT")
    );

    pageFailureRegistry.clearPage(props);

    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(paintBufferToCanvas).toHaveBeenCalledOnce());
  });

  it("never requests a page of a quarantined document", async () => {
    pageFailureRegistry.record(
      { ...props, pageIndex: 0, surface: "render" },
      { code: "PDFIUM_DOCUMENT_QUARANTINED" },
    );
    render(Thumbnail, { props });
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(renderThumbnail).not.toHaveBeenCalled();
  });
});
