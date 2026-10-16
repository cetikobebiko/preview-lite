/**
 * Pinch-zoom stress driver for the WebContent memory investigation (2026-10-13).
 *
 * Only built when VITE_ZOOM_STRESS=1 (see main.js). Once a PDF page is painted it sends the
 * ctrl+wheel events a trackpad pinch produces to `.viewer-area`, following a fixed schedule,
 * then closes the window. Run the build under scripts/memory-guard.py; phase times are relative
 * to the first painted page.
 */

const SCHEDULE = [
  { phase: "idle", waitMs: 8000 },
  { phase: "pinch-in-fast", events: 30, deltaY: -10, everyMs: 16, waitMs: 3000 },
  { phase: "pinch-out", events: 60, deltaY: 10, everyMs: 16, waitMs: 3000 },
  { phase: "pinch-in-slow", events: 30, deltaY: -10, everyMs: 50, waitMs: 3000 },
  { phase: "pinch-alternate", repeat: 6, events: 10, everyMs: 16, waitMs: 3000 },
  { phase: "idle-end", waitMs: 10000 },
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitForPaintedPage(doc) {
  while (!doc.querySelector(".pdf-page-container canvas")) await sleep(250);
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

async function burst(area, events, deltaY, everyMs) {
  for (let index = 0; index < events; index += 1) {
    pinch(area, deltaY);
    await sleep(everyMs);
  }
}

export async function runZoomStress({ doc = globalThis.document, closeWindow } = {}) {
  await waitForPaintedPage(doc);
  const started = performance.now();
  const area = doc.querySelector(".viewer-area");
  for (const step of SCHEDULE) {
    console.info(`[zoom-stress] ${((performance.now() - started) / 1000).toFixed(1)}s ${step.phase}`);
    if (step.repeat) {
      for (let round = 0; round < step.repeat; round += 1) {
        await burst(area, step.events, -10, step.everyMs);
        await burst(area, step.events, 10, step.everyMs);
      }
    } else if (step.events) {
      await burst(area, step.events, step.deltaY, step.everyMs);
    }
    await sleep(step.waitMs);
  }
  await closeWindow?.();
}
