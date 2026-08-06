import { afterEach, describe, expect, it, vi } from "vitest";
import { createSearchCoordinator } from "./searchCoordinator.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function setup(search = vi.fn()) {
  const context = {
    mounted: true,
    isPdf: true,
    sessionId: "session-search",
    contentGeneration: 1,
    scale: 1,
    rotation: 0,
    colorMode: "normal",
  };
  const events = [];
  const coordinator = createSearchCoordinator({
    search,
    getContext: () => context,
    onReset: (snapshot) => events.push(["reset", snapshot]),
    onSearching: (snapshot) => events.push(["searching", snapshot]),
    onResult: (result, snapshot) => events.push(["result", result, snapshot]),
    onError: (error, snapshot) => events.push(["error", error, snapshot]),
    onSettled: (snapshot) => events.push(["settled", snapshot]),
  });
  return { context, events, coordinator, search };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("search coordinator", () => {
  it("boş sorguda IPC çağrısı yapmaz", async () => {
    vi.useFakeTimers();
    const state = setup(vi.fn());
    state.coordinator.setQuery("   ");
    await vi.runAllTimersAsync();
    expect(state.search).not.toHaveBeenCalled();
    expect(state.events.map(([type]) => type)).toEqual(["reset"]);
  });

  it("hızlı sorgu değişiminde eski sonucu ve eski finally bloğunu düşürür", async () => {
    vi.useFakeTimers();
    const first = deferred();
    const second = deferred();
    const search = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const state = setup(search);

    state.coordinator.setQuery("first");
    await vi.advanceTimersByTimeAsync(300);
    state.coordinator.setQuery("second");
    await vi.advanceTimersByTimeAsync(300);

    first.resolve({ matches: [{ value: "stale" }] });
    await Promise.resolve();
    expect(state.events.filter(([type]) => type === "result")).toHaveLength(0);
    expect(state.events.filter(([type]) => type === "settled")).toHaveLength(0);

    second.resolve({ matches: [{ value: "fresh" }] });
    await Promise.resolve();
    await Promise.resolve();
    expect(state.events.filter(([type]) => type === "result")).toHaveLength(1);
    expect(state.events.filter(([type]) => type === "settled")).toHaveLength(1);
  });

  it.each([
    ["sessionId", "other-session"],
    ["contentGeneration", 2],
    ["mounted", false],
  ])("%s değiştiğinde geciken sonucu düşürür", async (field, value) => {
    vi.useFakeTimers();
    const request = deferred();
    const state = setup(vi.fn(() => request.promise));
    state.coordinator.setQuery("query");
    await vi.advanceTimersByTimeAsync(300);

    state.context[field] = value;
    request.resolve({ matches: [{ value: "stale" }] });
    await Promise.resolve();
    await Promise.resolve();

    expect(state.events.filter(([type]) => type === "result")).toHaveLength(0);
    expect(state.events.filter(([type]) => type === "settled")).toHaveLength(0);
  });

  it("güncel IPC hatasını raporlar ve güncel loading durumunu kapatır", async () => {
    vi.useFakeTimers();
    const state = setup(vi.fn(async () => {
      throw new Error("ipc failed");
    }));

    state.coordinator.setQuery("query");
    await vi.runAllTimersAsync();

    expect(state.events.filter(([type]) => type === "error")).toHaveLength(1);
    expect(state.events.filter(([type]) => type === "settled")).toHaveLength(1);
  });

  it("Rust iptal sonucunu uygulamaz", async () => {
    vi.useFakeTimers();
    const state = setup(vi.fn(async () => ({ is_cancelled: true, matches: [] })));
    state.coordinator.setQuery("query");
    await vi.runAllTimersAsync();
    expect(state.events.filter(([type]) => type === "result")).toHaveLength(0);
    expect(state.events.filter(([type]) => type === "settled")).toHaveLength(1);
  });

  it("scale, rotasyon ve renk değişiminde yeni arama başlatmaz", async () => {
    vi.useFakeTimers();
    const search = vi.fn(async () => ({ matches: [] }));
    const state = setup(search);
    state.coordinator.setQuery("query");
    await vi.runAllTimersAsync();
    expect(search).toHaveBeenCalledOnce();

    state.context.scale = 2;
    state.context.rotation = 90;
    state.context.colorMode = "dark";
    await vi.runAllTimersAsync();
    expect(search).toHaveBeenCalledOnce();
  });
});
