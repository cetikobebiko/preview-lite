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

// Observers keyed by rootMargin, so a test can move a thumbnail in and out of
// the load zone and the release zone separately.
function trackIntersectionObservers() {
  const observers = [];
  globalThis.IntersectionObserver = class IntersectionObserverMock {
    constructor(callback, options = {}) {
      this.callback = callback;
      this.options = options;
      observers.push(this);
    }
    observe(element) {
      this.element = element;
      this.callback([{ isIntersecting: true, target: element }]);
    }
    unobserve() {}
    disconnect() {}
  };
  return (rootMargin, isIntersecting) => {
    for (const observer of observers) {
      if (observer.options.rootMargin === rootMargin) {
        observer.callback([{ isIntersecting, target: observer.element }]);
      }
    }
  };
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

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
      }, "destroy")
    );
    expect(canvas.width).toBe(0);
    expect(canvas.height).toBe(0);
  });
});

describe("organizer thumbnail retention", () => {
  const props = { sessionId: "document-1", contentGeneration: 4, pageIndex: 3, displayIndex: 3 };

  it("keeps a painted thumbnail near the grid and releases it far away", async () => {
    const move = trackIntersectionObservers();
    const { container } = render(ThumbnailItem, { props });
    await waitFor(() => expect(paintBufferToCanvas).toHaveBeenCalledOnce());
    await waitFor(() => expect(container.querySelector(".thumb-placeholder")).toBeNull());

    move("100px", false);
    move("100px", true);
    await sleep(200);
    expect(renderThumbnail).toHaveBeenCalledOnce();
    expect(container.querySelector(".thumb-placeholder")).toBeNull();

    move("100px", false);
    move("1000px", false);
    await waitFor(() => expect(container.querySelector(".thumb-placeholder")).not.toBeNull());
    move("1000px", true);
    move("100px", true);
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledTimes(2));
  });

  it("cancels a render still in flight when it leaves the grid viewport", async () => {
    const move = trackIntersectionObservers();
    renderThumbnail.mockReturnValueOnce(new Promise(() => {}));
    render(ThumbnailItem, { props });
    await waitFor(() => expect(renderThumbnail).toHaveBeenCalledOnce());

    move("100px", false);

    await waitFor(() =>
      expect(cancelRenderRequest).toHaveBeenCalledWith(
        expect.objectContaining({ requestId: renderThumbnail.mock.calls[0][0].requestId }),
        "viewport_exit",
      )
    );
  });
});
