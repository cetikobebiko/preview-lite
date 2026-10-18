import { afterEach, describe, expect, it, vi } from "vitest";
import { confirm } from "@tauri-apps/plugin-dialog";
import * as sessionBackend from "./services/backend/session.js";
import * as pdfState from "./pdfState.js";
import { isSignatureLossDeclined } from "./services/signatureLossGuard.js";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("./services/backend/session.js", () => ({
  getSessionFormInfo: vi.fn(async () => ({ form_type: "AcroForm", signature_count: 1 })),
  rotateSessionPage: vi.fn(async () => ({ contentGeneration: 2 })),
  deleteSessionPages: vi.fn(async () => ({ contentGeneration: 2 })),
  reorderSessionPages: vi.fn(async () => ({ contentGeneration: 2 })),
  insertSessionBlankPage: vi.fn(async () => ({ contentGeneration: 2 })),
  addSessionAnnotation: vi.fn(async () => ({ contentGeneration: 2 })),
  deleteSessionAnnotation: vi.fn(async () => ({ contentGeneration: 2 })),
}));

afterEach(() => {
  confirm.mockReset();
  vi.clearAllMocks();
});

const runnable = { build: "development", platform: "macos" };
// The guards remember decisions per session, so every case uses its own session id.
// insertBlankPage is guarded too, but its `page_insert_blank` feature is disabled, so the
// feature gate rejects it before the guard runs.
const incrementalCases = [
  ["rotatePage", "rotateSessionPage", (id) => pdfState.rotatePage(id, 1, 0, 90)],
  ["deletePages", "deleteSessionPages", (id) => pdfState.deletePages(id, 1, [0])],
  [
    "addAnnotation",
    "addSessionAnnotation",
    (id) => pdfState.addAnnotation(id, 1, 0, "Highlight", { x: 0, y: 0, width: 0.1, height: 0.1 }, "#ffcc00", null, runnable),
  ],
  ["deleteAnnotation", "deleteSessionAnnotation", (id) => pdfState.deleteAnnotation(id, 1, 0, "0", runnable)],
];
const rewriteCases = [
  ["reorderPages", "reorderSessionPages", (id) => pdfState.reorderPages(id, 1, [1, 0])],
];

function expectDeclinedBeforeBackend() {
  return async (name, backend, run) => {
    confirm.mockResolvedValueOnce(false);
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const error = await run(`declined-${name}`).catch((caught) => caught);
    expect(isSignatureLossDeclined(error)).toBe(true);
    expect(sessionBackend[backend]).not.toHaveBeenCalled();
    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  };
}

describe("incremental mutations on a signed document", () => {
  it.each(incrementalCases)("%s does not reach the backend when the notice is declined", expectDeclinedBeforeBackend());

  it.each(incrementalCases)("%s runs after the notice is accepted", async (name, backend, run) => {
    confirm.mockResolvedValueOnce(true);
    await expect(run(`accepted-${name}`)).resolves.toEqual({ contentGeneration: 2 });
    expect(sessionBackend[backend]).toHaveBeenCalledTimes(1);
    const message = confirm.mock.calls[0][0];
    expect(message).toContain("imzalı içerik korunur");
    expect(message).toContain("değiştirilmiş olarak gösterir");
    expect(message).not.toContain("geçersiz");
  });
});

describe("full-rewrite mutations on a signed document", () => {
  it.each(rewriteCases)("%s does not reach the backend when the warning is declined", expectDeclinedBeforeBackend());

  it.each(rewriteCases)("%s runs after the warning is accepted", async (name, backend, run) => {
    confirm.mockResolvedValueOnce(true);
    await expect(run(`accepted-${name}`)).resolves.toEqual({ contentGeneration: 2 });
    expect(sessionBackend[backend]).toHaveBeenCalledTimes(1);
    expect(confirm.mock.calls[0][0]).toContain("imzaları geçersiz kılar");
  });

  it("still warns after the incremental notice was accepted in the same session", async () => {
    confirm.mockResolvedValue(true);
    await pdfState.rotatePage("mixed", 1, 0, 90);
    await pdfState.rotatePage("mixed", 2, 0, 90);
    await pdfState.reorderPages("mixed", 3, [1, 0]);
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(confirm.mock.calls[0][0]).toContain("imzalı içerik korunur");
    expect(confirm.mock.calls[1][0]).toContain("imzaları geçersiz kılar");
  });
});
