import { getFeaturePresentation, getRuntimeFeatureContext } from "../../featureRegistry.js";
import { describePluginError } from "../../pluginErrors.js";
import {
  checkPluginBackendGate,
  pluginExecutionHistory,
  refreshInstalledPlugins,
} from "../../pluginStore.js";
import * as pluginBackend from "../backend/plugins.js";
import {
  customCommandPayload,
  documentOpenPayload,
  exportCompletePayload,
  exportFormatFromExtension,
  validateHookPayload,
} from "./pluginHookPayloads.js";

const TERMINAL_STATES = new Set(["COMPLETED", "CANCELLED", "FAILED", "TIMED_OUT"]);
export const PLUGIN_HISTORY_LIMIT = 25;

/**
 * Shows plugin notifications as toasts no faster than one per `minIntervalMs`, keeping
 * at most `maxPending` waiting; the rest are dropped (they stay in the Plugin Manager's
 * execution list). The app's toast has no rate limit of its own, so plugins get this one.
 */
export function createNotificationLimiter({
  show, minIntervalMs = 1500, maxPending = 4,
  setTimer = (fn, ms) => setTimeout(fn, ms), clearTimer = (id) => clearTimeout(id),
} = {}) {
  const pending = [];
  let timer = null;

  function tick() {
    timer = null;
    const next = pending.shift();
    if (next === undefined) return;
    show(next);
    timer = setTimer(tick, minIntervalMs);
  }

  function push(message) {
    if (timer === null) {
      show(message);
      timer = setTimer(tick, minIntervalMs);
      return true;
    }
    if (pending.length >= maxPending) return false;
    pending.push(message);
    return true;
  }

  function reset() {
    pending.length = 0;
    if (timer !== null) clearTimer(timer);
    timer = null;
  }

  return Object.freeze({ push, reset });
}

function pluginsDeclaring(plugins, hookName) {
  return (Array.isArray(plugins) ? plugins : []).filter((plugin) => (
    plugin?.enabled === true
    && plugin?.consent_granted === true
    && Array.isArray(plugin.hooks)
    && plugin.hooks.includes(hookName)
  ));
}

/**
 * Runs plugin hooks for application events.
 *
 * - Callers never wait: every public entry returns at once and never throws.
 * - Only enabled, consented plugins whose manifest declares the hook are called,
 *   found with list_installed_plugins at dispatch time.
 * - One global FIFO queue: runs are strictly sequential, so a plugin never has more
 *   than one active execution.
 * - on_document_open carries the document identity (sessionId + contentGeneration);
 *   a run whose document changed is skipped, cancelled, or its result discarded.
 * - Frontend feature closed, backend gate closed or unknown → nothing happens, no error.
 * - Automatic runs report errors only in the execution history; runs the user started
 *   (`origin: "manual"`) also call `onError`.
 */
export function createPluginHookDispatcher({
  backend = pluginBackend,
  isFeatureRunnable = () => getFeaturePresentation("plugin_system", getRuntimeFeatureContext()).runnable,
  getDocumentContext = () => null,
  notify = () => {},
  history = pluginExecutionHistory,
  onPluginsChanged = () => {},
  newExecutionId = () => globalThis.crypto.randomUUID(),
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  now = () => Date.now(),
  pollIntervalMs = 150,
  maxWaitMs = 15000,
  maxQueue = 32,
} = {}) {
  const queue = [];
  let pumping = null;

  function record(entry) {
    history.update((entries) => {
      const index = entries.findIndex((item) => item.execution_id === entry.execution_id);
      if (index === -1) return [entry, ...entries].slice(0, PLUGIN_HISTORY_LIMIT);
      const next = entries.slice();
      next[index] = { ...next[index], ...entry };
      return next;
    });
  }

  async function isBackendGateOpen() {
    // Asked on every dispatch (one cheap IPC); any failure reads as closed.
    try {
      const status = await backend.getPluginFeatureStatus();
      return status?.enabled === true;
    } catch {
      return false;
    }
  }

  function sameDocument(a, b) {
    return !!a && !!b && a.sessionId === b.sessionId && a.contentGeneration === b.contentGeneration;
  }

  function isStale(job) {
    return job.documentContext !== null && !sameDocument(job.documentContext, getDocumentContext());
  }

  function enqueue(job) {
    if (job.supersedes) {
      // A newer document-open replaces queued ones for the same plugin.
      for (let i = queue.length - 1; i >= 0; i -= 1) {
        if (queue[i].plugin.id === job.plugin.id && queue[i].hookName === job.hookName) {
          queue[i].resolve();
          queue.splice(i, 1);
        }
      }
    }
    if (queue.length >= maxQueue) {
      job.resolve();
      return;
    }
    queue.push(job);
  }

  async function pump() {
    while (queue.length > 0) {
      const job = queue.shift();
      try {
        await runJob(job);
      } catch {
        // runJob records its own failures; never let one job stop the queue.
      }
      job.resolve();
    }
    pumping = null;
  }

  function reportManual(job, error) {
    if (job.origin === "manual" && typeof job.onError === "function") job.onError(error);
  }

  async function runJob(job) {
    const base = {
      plugin_id: job.plugin.id,
      plugin_name: job.plugin.name || job.plugin.id,
      hook_name: job.hookName,
      origin: job.origin,
      started_at: now(),
      result: null,
      error: null,
    };
    if (isStale(job)) return;

    const executionId = newExecutionId();
    record({ ...base, execution_id: executionId, state: "RUNNING" });
    try {
      await backend.startPluginExecution(job.plugin.id, job.hookName, job.payloadJson, executionId);
    } catch (error) {
      const wire = { code: error?.code ?? error?.cause?.code ?? null, message: describePluginError(error) };
      record({ ...base, execution_id: executionId, state: "FAILED", error: wire });
      reportManual(job, error);
      onPluginsChanged();
      return;
    }

    const deadline = now() + maxWaitMs;
    let status = null;
    let cancelledForStale = false;
    while (now() < deadline) {
      await sleep(pollIntervalMs);
      try {
        status = await backend.getPluginExecutionStatus(executionId);
      } catch {
        status = null;
        break;
      }
      if (TERMINAL_STATES.has(status?.state)) break;
      if (!cancelledForStale && isStale(job)) {
        cancelledForStale = true;
        backend.cancelPluginExecution(executionId).catch(() => {});
      }
    }

    if (isStale(job)) {
      record({ ...base, execution_id: executionId, state: "DISCARDED", result: null, error: null });
      return;
    }
    if (!status || !TERMINAL_STATES.has(status.state)) {
      const error = { code: null, message: "Eklenti yürütme durumu alınamadı." };
      record({ ...base, execution_id: executionId, state: "FAILED", error });
      reportManual(job, error);
      return;
    }

    record({ ...base, execution_id: executionId, state: status.state, result: status.result ?? null, error: status.error ?? null });
    if (status.state === "COMPLETED") {
      for (const note of status.result?.notifications ?? []) {
        notify(`Eklenti (${base.plugin_name}): ${note}`);
      }
      return;
    }
    reportManual(job, status.error ?? { code: null, message: status.state });
    // A failed run may have quarantined the plugin.
    onPluginsChanged();
  }

  /**
   * Queues `hookName` for every matching plugin. Resolves when those runs are done
   * (tests and the Plugin Manager may await it; app flows must not).
   */
  async function dispatch(hookName, payload, {
    origin = "auto", pluginIds = null, documentContext = null, onError = null,
  } = {}) {
    try {
      if (!isFeatureRunnable()) return;
      const checked = validateHookPayload(hookName, payload);
      if (!checked.ok) {
        if (origin === "manual" && typeof onError === "function") onError(new Error(checked.error));
        return;
      }
      if (!(await isBackendGateOpen())) return;
      let plugins;
      try {
        plugins = await backend.listInstalledPlugins();
      } catch {
        return;
      }
      let targets = pluginsDeclaring(plugins, hookName);
      if (Array.isArray(pluginIds)) targets = targets.filter((plugin) => pluginIds.includes(plugin.id));
      if (targets.length === 0) return;

      const payloadJson = JSON.stringify(checked.payload);
      const done = targets.map((plugin) => new Promise((resolve) => {
        enqueue({
          plugin, hookName, payloadJson, origin, onError,
          documentContext: documentContext ? { ...documentContext } : null,
          supersedes: hookName === "on_document_open",
          resolve,
        });
      }));
      if (!pumping) pumping = pump();
      await Promise.all(done);
    } catch {
      // Fire-and-forget contract: never surface dispatcher failures to app flows.
    }
  }

  function fire(hookName, payload, options) {
    // Detach from the caller's stack so even the synchronous part runs later.
    void Promise.resolve().then(() => dispatch(hookName, payload, options));
  }

  return Object.freeze({
    dispatch,
    documentOpened({ sessionId, contentGeneration, pageCount }) {
      if (!sessionId || !Number.isInteger(pageCount) || pageCount < 1) return;
      fire("on_document_open", documentOpenPayload(pageCount), {
        documentContext: { sessionId, contentGeneration },
      });
    },
    exportCompleted(format, pageCount) {
      const v1 = exportFormatFromExtension(format);
      if (!v1 || !Number.isInteger(pageCount) || pageCount < 1) return;
      fire("on_export_complete", exportCompletePayload(v1, pageCount));
    },
    runCustomCommand(pluginId, { onError } = {}) {
      fire("on_custom_command", customCommandPayload(), { origin: "manual", pluginIds: [pluginId], onError });
    },
    queueLength: () => queue.length,
  });
}

// ---------------------------------------------------------------------------
// App-wide instance. App.svelte calls configurePluginHookDispatcher once with the
// document context and the toast; flows call the helpers below (fire-and-forget).
// ---------------------------------------------------------------------------

const appBindings = {
  getDocumentContext: () => null,
  showToast: () => {},
};

let limiter = createNotificationLimiter({ show: (message) => appBindings.showToast(message) });

const appDispatcher = createPluginHookDispatcher({
  getDocumentContext: () => appBindings.getDocumentContext(),
  notify: (message) => limiter.push(message),
  onPluginsChanged: () => { refreshInstalledPlugins().catch(() => {}); },
});

export function configurePluginHookDispatcher({ getDocumentContext, showToast } = {}) {
  if (typeof getDocumentContext === "function") appBindings.getDocumentContext = getDocumentContext;
  if (typeof showToast === "function") appBindings.showToast = showToast;
  limiter.reset();
  limiter = createNotificationLimiter({ show: (message) => appBindings.showToast(message) });
}

export const pluginHookDispatcher = appDispatcher;

export function notifyPluginDocumentOpened(detail) {
  try { appDispatcher.documentOpened(detail); } catch { /* fire-and-forget */ }
}

export function notifyPluginExportCompleted(format, pageCount) {
  try { appDispatcher.exportCompleted(format, pageCount); } catch { /* fire-and-forget */ }
}

export function runPluginCustomCommand(pluginId, options) {
  try { appDispatcher.runCustomCommand(pluginId, options); } catch { /* fire-and-forget */ }
}

/**
 * Loads the plugin list (for the command palette) when the plugin system can run.
 * Silent when the feature or the backend gate is closed.
 */
export async function primePluginCommands({ check = checkPluginBackendGate, refresh = refreshInstalledPlugins } = {}) {
  try {
    if (!getFeaturePresentation("plugin_system", getRuntimeFeatureContext()).runnable) return;
    const gate = await check();
    if (!gate?.enabled) return;
    await refresh();
  } catch {
    // Silent: no plugin commands.
  }
}
