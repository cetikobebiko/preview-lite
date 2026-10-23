/**
 * Worker cancellation baseline driver (S1–S5, audit item 14, 2026-10-23).
 *
 * Only built when VITE_CANCEL_BASELINE=1 (see main.js). Once the first PDF page is painted it
 * plays a fixed schedule of user-like input against the open document, then closes the window:
 *
 *   S1 scroll            fast then slow wheel-like scrolling of the page list
 *   S2 zoom              pinch bursts and Cmd+=/Cmd+- steps
 *   S3 thumbnails        fast scrolling of the thumbnail sidebar
 *   S4 search+scroll     superseding searches while the page list scrolls
 *   S5 idle              no input; background work only
 *
 * Every phase is followed by a settle gap so events can be attributed by time. The webview's
 * console is not captured in release builds, so scripts/cancel-baseline-report.mjs slices the
 * pdfium_worker_metrics log with the same schedule, anchored at the first rendered page.
 * Run the build with an isolated HOME (see docs/engineering/pdfium-worker-isolation.md).
 */

export const SETTLE_MS = 5000;

export const SCHEDULE = Object.freeze([
  { phase: "warmup", durationMs: 8000 },
  { phase: "S1-scroll", durationMs: 25000 },
  { phase: "S2-zoom", durationMs: 25000 },
  { phase: "S3-thumbnails", durationMs: 20000 },
  { phase: "S4-search-scroll", durationMs: 25000 },
  { phase: "S5-idle", durationMs: 20000 },
]);

/** [phase, startMs, endMs] relative to the first painted page, settle gaps excluded. */
export function phaseWindows(schedule = SCHEDULE, settleMs = SETTLE_MS) {
  const windows = [];
  let at = 0;
  for (const step of schedule) {
    windows.push([step.phase, at, at + step.durationMs]);
    at += step.durationMs + settleMs;
  }
  return windows;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(doc, selector) {
  while (!doc.querySelector(selector)) await sleep(250);
  return doc.querySelector(selector);
}

async function forDuration(durationMs, everyMs, step) {
  const end = performance.now() + durationMs;
  let index = 0;
  while (performance.now() < end) {
    step(index);
    index += 1;
    await sleep(everyMs);
  }
}

function key(target, keyName, modifiers = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", {
    key: keyName,
    metaKey: true,
    bubbles: true,
    cancelable: true,
    ...modifiers,
  }));
}

function pinch(area, deltaY) {
  const rect = area.getBoundingClientRect();
  area.dispatchEvent(new WheelEvent("wheel", {
    deltaY,
    ctrlKey: true,
    clientX: rect.left + rect.width / 2,
    clientY: rect.top + rect.height / 2,
    bubbles: true,
    cancelable: true,
  }));
}

async function scrollPages(area, durationMs) {
  // Fast flicks for 60% of the phase, then slow reading-speed scrolling.
  const fastMs = Math.round(durationMs * 0.6);
  await forDuration(fastMs, 16, () => area.scrollBy({ top: 900 }));
  await forDuration(durationMs - fastMs, 120, () => area.scrollBy({ top: 250 }));
}

async function zoom(win, area, durationMs) {
  await forDuration(durationMs, 600, (index) => {
    if (index % 3 === 2) {
      key(win, index % 2 ? "=" : "-");
      return;
    }
    for (let event = 0; event < 8; event += 1) pinch(area, index % 2 ? 10 : -10);
  });
}

async function scrollThumbnails(doc, win, durationMs) {
  if (!doc.querySelector(".sidebar")) key(win, "b");
  const list = await waitFor(doc, ".sidebar .overflow-y-auto");
  // Fast flicks never outlast the thumbnail dwell (120 ms), so they request almost nothing.
  // The stepped part pauses long enough for requests to start and then moves on, which is
  // what cancels in-flight thumbnails.
  const fastMs = Math.round(durationMs * 0.4);
  await forDuration(fastMs, 16, () => list.scrollBy({ top: 700 }));
  await forDuration(durationMs - fastMs, 250, () => list.scrollBy({ top: 500 }));
  key(win, "b");
}

function setSearch(input, value) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

async function searchWhileScrolling(doc, win, area, durationMs) {
  key(win, "f");
  const input = await waitFor(doc, 'input[aria-label="Belgede ara"]');
  const queries = ["needle", "lorem", "needle dolor", "ipsum", "needle"];
  const every = Math.floor(durationMs / queries.length);
  const scrolling = forDuration(durationMs, 40, () => area.scrollBy({ top: 400 }));
  for (const query of queries) {
    setSearch(input, query);
    await sleep(every);
  }
  await scrolling;
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
}

export async function runCancelBaseline({
  doc = globalThis.document,
  win = globalThis.window,
  closeWindow,
} = {}) {
  // WKWebView keeps localStorage under the real user's Library even with an isolated HOME, so
  // the viewer settings the zoom phase writes are put back before the window closes.
  const savedSettings = readSetting("antigravity_settings");
  await waitFor(doc, ".pdf-page-container canvas");
  const area = doc.querySelector(".viewer-area");
  const actions = {
    warmup: (ms) => sleep(ms),
    "S1-scroll": (ms) => scrollPages(area, ms),
    "S2-zoom": (ms) => zoom(win, area, ms),
    "S3-thumbnails": (ms) => scrollThumbnails(doc, win, ms),
    "S4-search-scroll": (ms) => searchWhileScrolling(doc, win, area, ms),
    "S5-idle": (ms) => sleep(ms),
  };
  const started = performance.now();
  for (const [phase, startMs, endMs] of phaseWindows()) {
    // Re-anchor to the schedule so a slow phase does not shift later windows.
    await sleep(Math.max(0, startMs - (performance.now() - started)));
    console.info(`[cancel-baseline] ${((performance.now() - started) / 1000).toFixed(1)}s ${phase}`);
    await Promise.race([actions[phase](endMs - startMs), sleep(endMs - startMs + 1000)]);
  }
  await sleep(SETTLE_MS);
  writeSetting("antigravity_settings", savedSettings);
  await closeWindow?.();
}

function readSetting(name) {
  try {
    return globalThis.localStorage?.getItem(name) ?? null;
  } catch {
    return null;
  }
}

function writeSetting(name, value) {
  try {
    if (value === null) globalThis.localStorage?.removeItem(name);
    else globalThis.localStorage?.setItem(name, value);
  } catch {
    // Storage unavailable: nothing was persisted either.
  }
}
