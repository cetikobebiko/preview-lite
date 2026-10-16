import { describe, expect, it, vi } from "vitest";
import { createPageSizeStore } from "./pageSizeStore.js";

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("createPageSizeStore", () => {
  it("asks the worker once per session and content generation", async () => {
    const load = vi.fn(async () => [{ widthPt: 612, heightPt: 792 }, { widthPt: 842, heightPt: 595 }]);
    const store = createPageSizeStore({ load });
    const sizes = await Promise.all([
      store.requestPage("s1", 1, 0),
      store.requestPage("s1", 1, 1),
      store.requestPage("s1", 1, 5),
    ]);
    expect(sizes).toEqual([{ widthPt: 612, heightPt: 792 }, { widthPt: 842, heightPt: 595 }, null]);
    expect(store.peek("s1", 1, 1)).toEqual({ widthPt: 842, heightPt: 595 });
    expect(load).toHaveBeenCalledTimes(1);

    await store.requestPage("s1", 2, 0);
    expect(load).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenLastCalledWith("s1", 2);
    expect(store.peek("s1", 1, 0)).toBeNull();
  });

  it("ignores a late answer for a document that is no longer current", async () => {
    const stale = createDeferred();
    const load = vi.fn()
      .mockImplementationOnce(() => stale.promise)
      .mockImplementationOnce(async () => [{ widthPt: 100, heightPt: 200 }]);
    const store = createPageSizeStore({ load });
    const staleRequest = store.requestPage("old", 1, 0);
    await expect(store.requestPage("new", 1, 0)).resolves.toEqual({ widthPt: 100, heightPt: 200 });
    stale.resolve([{ widthPt: 1, heightPt: 1 }]);
    await expect(staleRequest).resolves.toBeNull();
    expect(store.peek("new", 1, 0)).toEqual({ widthPt: 100, heightPt: 200 });
    expect(store.peek("old", 1, 0)).toBeNull();
  });

  it("resolves to null without retrying when the worker call fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const load = vi.fn(async () => { throw new Error("STALE_GENERATION"); });
    const store = createPageSizeStore({ load });
    await expect(store.requestPage("s1", 1, 0)).resolves.toBeNull();
    await expect(store.requestPage("s1", 1, 1)).resolves.toBeNull();
    expect(load).toHaveBeenCalledTimes(1);
    expect(store.peek("s1", 1, 0)).toBeNull();
    warn.mockRestore();
  });

  it("does nothing without a session", async () => {
    const load = vi.fn();
    const store = createPageSizeStore({ load });
    await expect(store.requestPage("", 1, 0)).resolves.toBeNull();
    expect(load).not.toHaveBeenCalled();
  });

  it("hands out every loaded size of the current document only", async () => {
    const store = createPageSizeStore({ load: async () => [{ widthPt: 612, heightPt: 792 }, null] });
    expect(store.peekAll("s1", 1)).toBeNull();
    await store.request("s1", 1);
    expect(store.peekAll("s1", 1)).toEqual([{ widthPt: 612, heightPt: 792 }, null]);
    expect(store.peekAll("s1", 1)).toBe(store.peekAll("s1", 1));
    expect(store.peekAll("s1", 2)).toBeNull();
    expect(store.peekAll("", 1)).toBeNull();
  });
});
