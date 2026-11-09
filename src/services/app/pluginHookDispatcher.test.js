import { describe, expect, it, vi } from "vitest";
import { get, writable } from "svelte/store";
import { createNotificationLimiter, createPluginHookDispatcher } from "./pluginHookDispatcher.js";

const plugin = (id, hooks, extra = {}) => ({
  id, name: id.toUpperCase(), hooks, enabled: true, consent_granted: true, ...extra,
});

function completed(notifications = [], logs = []) {
  return { state: "COMPLETED", result: { notifications, logs, warnings: [], text: null }, error: null };
}

/** Backend double: each started execution finishes after `ticks` status polls. */
function fakeBackend({ plugins = [], gate = true, outcome = () => completed(), ticks = 1 } = {}) {
  const runs = new Map();
  let active = 0;
  const backend = {
    maxActive: 0,
    getPluginFeatureStatus: vi.fn(async () => ({ enabled: gate })),
    listInstalledPlugins: vi.fn(async () => plugins),
    startPluginExecution: vi.fn(async (pluginId, hook, payloadJson, executionId) => {
      active += 1;
      backend.maxActive = Math.max(backend.maxActive, active);
      runs.set(executionId, { pluginId, hook, payload: JSON.parse(payloadJson), polls: 0 });
      return { execution_id: executionId, plugin_id: pluginId, state: "RUNNING" };
    }),
    getPluginExecutionStatus: vi.fn(async (executionId) => {
      const run = runs.get(executionId);
      run.polls += 1;
      if (run.polls < ticks) return { state: "RUNNING" };
      active -= 1;
      return { execution_id: executionId, ...outcome(run) };
    }),
    cancelPluginExecution: vi.fn(async () => {}),
  };
  return backend;
}

function setup(options = {}) {
  const backend = fakeBackend(options);
  const history = writable([]);
  const notify = vi.fn();
  let n = 0;
  let documentContext = options.documentContext ?? { sessionId: "s1", contentGeneration: 1 };
  const dispatcher = createPluginHookDispatcher({
    backend,
    history,
    notify,
    isFeatureRunnable: () => options.featureRunnable ?? true,
    getDocumentContext: () => documentContext,
    newExecutionId: () => `e${++n}`,
    sleep: () => Promise.resolve(),
    onPluginsChanged: options.onPluginsChanged ?? (() => {}),
  });
  return {
    backend, history, notify, dispatcher,
    setDocument: (ctx) => { documentContext = ctx; },
  };
}

describe("pluginHookDispatcher", () => {
  it("calls only enabled, consented plugins that declare the hook", async () => {
    const { backend, dispatcher } = setup({
      plugins: [
        plugin("a", ["on_export_complete"]),
        plugin("b", ["on_document_open"]),
        plugin("c", ["on_export_complete"], { enabled: false }),
        plugin("d", ["on_export_complete"], { consent_granted: false }),
        plugin("e", ["on_custom_command", "on_export_complete"]),
      ],
    });
    await dispatcher.dispatch("on_export_complete", { schema_version: 1, format: "docx", page_count: 4 });
    const calls = backend.startPluginExecution.mock.calls;
    expect(calls.map(([id]) => id)).toEqual(["a", "e"]);
    for (const [, hook, payload] of calls) {
      expect(hook).toBe("on_export_complete");
      expect(JSON.parse(payload)).toEqual({ schema_version: 1, format: "docx", page_count: 4 });
    }
  });

  it("runs sequentially: never more than one active execution", async () => {
    const { backend, dispatcher } = setup({
      plugins: [plugin("a", ["on_export_complete"]), plugin("b", ["on_export_complete"])],
      ticks: 3,
    });
    await Promise.all([
      dispatcher.dispatch("on_export_complete", { schema_version: 1, format: "pdf", page_count: 1 }),
      dispatcher.dispatch("on_export_complete", { schema_version: 1, format: "md", page_count: 2 }),
    ]);
    expect(backend.startPluginExecution).toHaveBeenCalledTimes(4);
    expect(backend.maxActive).toBe(1);
  });

  it("does nothing when the frontend feature or the backend gate is closed", async () => {
    for (const options of [{ featureRunnable: false }, { gate: false }]) {
      const { backend, dispatcher, history, notify } = setup({ plugins: [plugin("a", ["on_document_open"])], ...options });
      await dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 3 }, {
        documentContext: { sessionId: "s1", contentGeneration: 1 },
      });
      expect(backend.startPluginExecution).not.toHaveBeenCalled();
      expect(backend.listInstalledPlugins).not.toHaveBeenCalled();
      expect(get(history)).toEqual([]);
      expect(notify).not.toHaveBeenCalled();
    }
    // A failing gate query also counts as closed, silently.
    const { backend, dispatcher } = setup({ plugins: [plugin("a", ["on_document_open"])] });
    backend.getPluginFeatureStatus.mockRejectedValue(new Error("ipc down"));
    await expect(dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 3 })).resolves.toBeUndefined();
    expect(backend.startPluginExecution).not.toHaveBeenCalled();
  });

  it("refuses a payload outside the schema before calling the backend", async () => {
    const { backend, dispatcher } = setup({ plugins: [plugin("a", ["on_document_open"])] });
    const onError = vi.fn();
    await dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 3, file_name: "x.pdf" }, { origin: "manual", onError });
    expect(backend.startPluginExecution).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it("toasts notifications of a completed run, prefixed with the plugin name", async () => {
    const { dispatcher, notify, history } = setup({
      plugins: [plugin("hello", ["on_document_open"])],
      outcome: (run) => completed([`belge açıldı, ${run.payload.page_count} sayfa.`], ["log line"]),
    });
    await dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 7 }, {
      documentContext: { sessionId: "s1", contentGeneration: 1 },
    });
    expect(notify).toHaveBeenCalledWith("Eklenti (HELLO): belge açıldı, 7 sayfa.");
    const [entry] = get(history);
    expect(entry).toMatchObject({ plugin_id: "hello", hook_name: "on_document_open", origin: "auto", state: "COMPLETED" });
    expect(entry.result.logs).toEqual(["log line"]);
  });

  it("discards an on_document_open result when the document changed meanwhile", async () => {
    const { dispatcher, notify, history, backend, setDocument } = setup({
      plugins: [plugin("hello", ["on_document_open"])],
      ticks: 2,
      outcome: () => completed(["eski belge"]),
    });
    backend.getPluginExecutionStatus.mockImplementationOnce(async () => {
      setDocument({ sessionId: "s2", contentGeneration: 1 }); // user switched documents
      return { state: "RUNNING" };
    });
    await dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 3 }, {
      documentContext: { sessionId: "s1", contentGeneration: 1 },
    });
    expect(notify).not.toHaveBeenCalled();
    expect(get(history)[0]).toMatchObject({ state: "DISCARDED", result: null });
    expect(backend.cancelPluginExecution).toHaveBeenCalledWith("e1");
  });

  it("skips a queued on_document_open for a document that is no longer current", async () => {
    const { dispatcher, backend, setDocument } = setup({ plugins: [plugin("hello", ["on_document_open"])] });
    setDocument({ sessionId: "s1", contentGeneration: 2 }); // a mutation bumped the generation
    await dispatcher.dispatch("on_document_open", { schema_version: 1, page_count: 3 }, {
      documentContext: { sessionId: "s1", contentGeneration: 1 },
    });
    expect(backend.startPluginExecution).not.toHaveBeenCalled();
  });

  it("keeps automatic failures in history only, reports manual ones", async () => {
    const failure = () => ({ state: "FAILED", result: null, error: { code: "PLUGIN_SCRIPT_RUNTIME_ERROR", message: "boom" } });
    const onPluginsChanged = vi.fn();
    const { dispatcher, history, notify } = setup({ plugins: [plugin("a", ["on_custom_command"])], outcome: failure, onPluginsChanged });
    const autoError = vi.fn();
    await dispatcher.dispatch("on_custom_command", { schema_version: 1 }, { onError: autoError });
    expect(autoError).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
    expect(get(history)[0]).toMatchObject({ state: "FAILED", origin: "auto" });
    expect(onPluginsChanged).toHaveBeenCalled();

    const manualError = vi.fn();
    await dispatcher.dispatch("on_custom_command", { schema_version: 1 }, { origin: "manual", onError: manualError });
    expect(manualError).toHaveBeenCalledWith({ code: "PLUGIN_SCRIPT_RUNTIME_ERROR", message: "boom" });
  });

  it("records a rejected start and reports it only for manual runs", async () => {
    const { dispatcher, backend, history } = setup({ plugins: [plugin("a", ["on_custom_command"])] });
    backend.startPluginExecution.mockRejectedValue({ code: "PLUGIN_EXECUTION_BUSY", message: "busy" });
    const onError = vi.fn();
    await dispatcher.dispatch("on_custom_command", { schema_version: 1 }, { onError });
    expect(onError).not.toHaveBeenCalled();
    expect(get(history)[0]).toMatchObject({ state: "FAILED", error: { code: "PLUGIN_EXECUTION_BUSY" } });
    await dispatcher.dispatch("on_custom_command", { schema_version: 1 }, { origin: "manual", onError });
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it("limits a custom command to the chosen plugin", async () => {
    const { dispatcher, backend } = setup({
      plugins: [plugin("a", ["on_custom_command"]), plugin("b", ["on_custom_command"])],
    });
    await dispatcher.dispatch("on_custom_command", { schema_version: 1 }, { origin: "manual", pluginIds: ["b"] });
    expect(backend.startPluginExecution.mock.calls.map(([id]) => id)).toEqual(["b"]);
  });

  it("fire-and-forget helpers return synchronously and never throw", async () => {
    const { dispatcher, backend } = setup({ plugins: [plugin("a", ["on_export_complete", "on_document_open"])] });
    backend.listInstalledPlugins.mockRejectedValue(new Error("boom"));
    expect(dispatcher.exportCompleted("jpg", 2)).toBeUndefined();
    expect(dispatcher.documentOpened({ sessionId: "s1", contentGeneration: 1, pageCount: 2 })).toBeUndefined();
    // Nothing ran on the caller's stack.
    expect(backend.getPluginFeatureStatus).not.toHaveBeenCalled();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backend.startPluginExecution).not.toHaveBeenCalled();
    // Invalid input is dropped before anything is queued.
    expect(dispatcher.exportCompleted("xlsx", 2)).toBeUndefined();
    expect(dispatcher.documentOpened({ sessionId: "s1", contentGeneration: 1, pageCount: 0 })).toBeUndefined();
  });

  it("exportCompleted normalises the format before dispatching", async () => {
    const { dispatcher, backend } = setup({ plugins: [plugin("a", ["on_export_complete"])] });
    dispatcher.exportCompleted("jpg", 3);
    await vi.waitFor(() => expect(backend.startPluginExecution).toHaveBeenCalled());
    expect(JSON.parse(backend.startPluginExecution.mock.calls[0][2])).toEqual({ schema_version: 1, format: "jpeg", page_count: 3 });
  });
});

describe("createNotificationLimiter", () => {
  it("shows one toast per interval and drops the overflow", () => {
    const shown = [];
    const timers = [];
    const limiter = createNotificationLimiter({
      show: (message) => shown.push(message),
      minIntervalMs: 1000,
      maxPending: 2,
      setTimer: (fn) => { timers.push(fn); return timers.length; },
      clearTimer: () => {},
    });
    expect(limiter.push("1")).toBe(true);
    expect(limiter.push("2")).toBe(true);
    expect(limiter.push("3")).toBe(true);
    expect(limiter.push("4")).toBe(false);
    expect(shown).toEqual(["1"]);
    timers.shift()();
    expect(shown).toEqual(["1", "2"]);
    timers.shift()();
    expect(shown).toEqual(["1", "2", "3"]);
    timers.shift()();
    expect(shown).toEqual(["1", "2", "3"]);
  });
});
