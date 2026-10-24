#!/usr/bin/env node
// Splits a pdfium_worker_metrics log from a VITE_CANCEL_BASELINE=1 run into the S1–S5 phases
// of src/devtools/cancelBaseline.js and summarizes each phase with worker-metrics-report.mjs.
//
// The driver cannot write phase markers to the Rust log, so phases are time windows anchored
// at the first successful render-page exchange (the driver starts once a page is painted).
//
//   node scripts/cancel-baseline-report.mjs <json-log>...

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { phaseWindows } from "../src/devtools/cancelBaseline.js";
import {
  WORKER_METRICS_TARGET,
  parseMetricEvents,
  summarizeWorkerMetrics,
} from "./worker-metrics-report.mjs";

export function anchorOf(events) {
  const first = events.find(
    (event) =>
      event.event === "exchange" &&
      event.command_class === "render-page" &&
      event.outcome === "ok" &&
      event.at !== null,
  );
  return first ? first.at : null;
}

export function splitByPhase(events, anchor, windows = phaseWindows()) {
  return windows.map(([phase, startMs, endMs]) => ({
    phase,
    events: events.filter(
      (event) => event.at !== null && event.at >= anchor + startMs && event.at < anchor + endMs,
    ),
  }));
}

/** The numbers the grace and chunk constants are judged by, per phase. */
export function phaseVerdict(summary) {
  const outcomes = summary.exchangeOutcomes;
  const sum = (suffix) =>
    Object.entries(outcomes)
      .filter(([key]) => key.endsWith(suffix))
      .reduce((total, [, count]) => total + count, 0);
  const drained = sum(":cancelled_drained");
  const killed = sum(":cancelled_killed");
  return {
    inFlightCancels: drained + killed,
    drained,
    killed,
    drainRate: drained + killed > 0 ? drained / (drained + killed) : null,
    replacements: Object.entries(summary.workerSpawns)
      .filter(([reason]) => reason !== "initial")
      .reduce((total, [, count]) => total + count, 0),
  };
}

function main(paths) {
  if (paths.length === 0) {
    console.error("usage: node scripts/cancel-baseline-report.mjs <json-log>...");
    process.exit(2);
  }
  const events = paths.flatMap((path) => parseMetricEvents(readFileSync(path, "utf8")));
  const anchor = anchorOf(events);
  if (anchor === null) {
    console.error(
      `No successful render-page exchange found. Was RUST_LOG=${WORKER_METRICS_TARGET}=debug set?`,
    );
    process.exit(1);
  }
  const report = splitByPhase(events, anchor).map(({ phase, events: phaseEvents }) => {
    const summary = summarizeWorkerMetrics(phaseEvents);
    return { phase, eventCount: phaseEvents.length, verdict: phaseVerdict(summary), summary };
  });
  console.log(JSON.stringify(report, null, 2));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
