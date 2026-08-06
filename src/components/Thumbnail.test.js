import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import {
  cancelRenderRequest,
  renderThumbnail,
} from "../services/backend/viewer.js";
import { paintBufferToCanvas } from "../utils/canvasUtils.js";
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
      })
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
