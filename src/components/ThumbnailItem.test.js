import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import {
  cancelRenderRequest,
  renderThumbnail,
} from "../services/backend/viewer.js";
import { paintBufferToCanvas } from "../utils/canvasUtils.js";
import ThumbnailItem from "./ThumbnailItem.svelte";

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

beforeEach(() => {
  globalThis.IntersectionObserver = class IntersectionObserverMock {
    constructor(callback) {
      this.callback = callback;
    }
    observe(element) {
      this.callback([{ isIntersecting: true, target: element }]);
    }
    unobserve() {}
    disconnect() {}
  };
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
});

describe("organizer thumbnail item RGBA pipeline", () => {
  it("keeps the existing 150px target and organizer scope", async () => {
    render(ThumbnailItem, {
      props: {
        sessionId: "document-1",
        contentGeneration: 4,
        pageIndex: 3,
        displayIndex: 3,
      },
    });

    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    expect(renderThumbnail).toHaveBeenCalledWith(expect.objectContaining({
      sessionId: "document-1",
      pageIndex: 3,
      targetWidth: 150,
      contentGeneration: 4,
      scope: "organizer_thumbnail",
      requestId: expect.any(String),
    }));
    await waitFor(() => expect(paintBufferToCanvas).toHaveBeenCalledOnce());
  });

  it("cancels the current request and releases the canvas on unmount", async () => {
    renderThumbnail.mockReturnValue(new Promise(() => {}));
    const { getByLabelText, unmount } = render(ThumbnailItem, {
      props: {
        sessionId: "document-1",
        contentGeneration: 4,
        pageIndex: 1,
        displayIndex: 1,
      },
    });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());
    const request = renderThumbnail.mock.calls[0][0];
    const canvas = getByLabelText("Sayfa 2 önizleme");

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
    expect(canvas.width).toBe(0);
    expect(canvas.height).toBe(0);
  });
});
