#!/usr/bin/env node
// Summarizes PDFium worker metrics from the release build's JSON log.
//
// Capture (release build; JSON lines are written to logs/antigravity.log.*):
//   RUST_LOG=pdfium_worker_metrics=debug <app>
// Report:
//   node scripts/worker-metrics-report.mjs logs/antigravity.log.2026-10-11
//
// Events carry only command classes, durations, epochs and causes (see
// WORKER_METRICS_TARGET in src-tauri/src/pdf_engine/subprocess_manager.rs).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const WORKER_METRICS_TARGET = "pdfium_worker_metrics";

export function parseMetricEvents(text) {
  const events = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      continue;
    }
    if (record?.target !== WORKER_METRICS_TARGET || !record.fields?.event) continue;
    const at = Date.parse(record.timestamp);
    events.push({ at: Number.isFinite(at) ? at : null, ...record.fields });
  }
  return events;
}

export function percentile(values, fraction) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1);
  return sorted[Math.max(0, index)];
}

function distribution(values) {
  return {
    count: values.length,
    p50: percentile(values, 0.5),
    p95: percentile(values, 0.95),
    p99: percentile(values, 0.99),
  };
}

function countBy(events, keyOf) {
  const counts = {};
  for (const event of events) {
    const key = keyOf(event);
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return counts;
}

function groupDurations(events, keyOf, field) {
  const groups = {};
  for (const event of events) {
    const value = Number(event[field]);
    if (!Number.isFinite(value)) continue;
    (groups[keyOf(event)] ??= []).push(value);
  }
  return Object.fromEntries(
    Object.entries(groups).map(([key, values]) => [key, distribution(values)]),
  );
}

export function summarizeWorkerMetrics(events) {
  const of = (name) => events.filter((event) => event.event === name);
  const timestamps = events.map((event) => event.at).filter((at) => at !== null);
  const spanMinutes =
    timestamps.length > 1
      ? (Math.max(...timestamps) - Math.min(...timestamps)) / 60_000
      : null;
  const spawns = of("worker_spawn");
  const replacements = spawns.filter((event) => event.reason !== "initial");
  const exchanges = of("exchange");

  return {
    spanMinutes,
    workerSpawns: countBy(spawns, (event) => event.reason),
    replacementsPerMinute:
      spanMinutes && spanMinutes > 0 ? replacements.length / spanMinutes : null,
    spawnToHelloMs: distribution(
      spawns.map((event) => Number(event.spawn_to_hello_ms)).filter(Number.isFinite),
    ),
    terminations: countBy(of("worker_terminate"), (event) => event.cause),
    exchangeOutcomes: countBy(
      exchanges,
      (event) => `${event.command_class}:${event.outcome}`,
    ),
    exchangeMsByClass: groupDurations(
      exchanges.filter((event) => event.outcome === "ok"),
      (event) => event.command_class,
      "duration_ms",
    ),
    renderCancels: countBy(
      of("render_cancel"),
      (event) =>
        [event.trigger, event.reason, event.scope].filter(Boolean).join(":"),
    ),
    cancelObservedIn: {
      scheduler_queue: of("render_cancel_observed").length,
      admission_queue: of("admission").filter(
        (event) => event.outcome === "cancelled_in_queue",
      ).length,
      in_flight_drained: exchanges.filter(
        (event) => event.outcome === "cancelled_drained",
      ).length,
      in_flight_killed: exchanges.filter(
        (event) => event.outcome === "cancelled_killed",
      ).length,
    },
    otherCancels: countBy(
      [...of("search_cancel"), ...of("text_layout_cancel")],
      (event) => event.trigger,
    ),
    admissionWaitMs: groupDurations(
      of("admission").filter((event) => event.outcome === "granted"),
      (event) => event.mode,
      "wait_ms",
    ),
    searches: {
      ...distribution(
        of("search_completed")
          .map((event) => Number(event.duration_ms))
          .filter(Number.isFinite),
      ),
      chunks: distribution(
        of("search_completed").map((event) => Number(event.chunks)).filter(Number.isFinite),
      ),
    },
  };
}

function main(paths) {
  if (paths.length === 0) {
    console.error("usage: node scripts/worker-metrics-report.mjs <json-log>...");
    process.exit(2);
  }
  const events = paths.flatMap((path) => parseMetricEvents(readFileSync(path, "utf8")));
  if (events.length === 0) {
    console.error(
      `No ${WORKER_METRICS_TARGET} events found. Was RUST_LOG=${WORKER_METRICS_TARGET}=debug set?`,
    );
    process.exit(1);
  }
  console.log(JSON.stringify(summarizeWorkerMetrics(events), null, 2));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
