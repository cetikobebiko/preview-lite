import { describe, expect, it, vi } from "vitest";
import { createFormCommitController, draftToEdit } from "./formCommitController.svelte.js";
import { createFormDraftState } from "./formDraftState.svelte.js";

vi.mock("../pdfState.js", () => ({ fillFormFields: vi.fn(), getFormInfo: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function setup({ signatureCount = 0, confirm = true } = {}) {
  const drafts = createFormDraftState();
  drafts.bindSession("session");
  const timers = [];
  const dependencies = {
    fill: vi.fn(),
    getFormInfo: vi.fn(async () => ({ form_type: "AcroForm", signature_count: signatureCount })),
    confirmSignatureInvalidation: vi.fn(async () => confirm),
    delay: (callback) => timers.push(callback),
    cancelDelay: () => {},
  };
  const controller = createFormCommitController(drafts, dependencies);
  const flush = async () => {
    while (timers.length) timers.shift()();
    for (let i = 0; i < 10; i += 1) await Promise.resolve();
  };
  return { drafts, dependencies, controller, flush };
}

const target = (annotIndex, pageIndex = 0) => ({ pageIndex, annotIndex });

describe("form commit controller", () => {
  it("maps every draft kind to the backend edit shape", () => {
    expect(draftToEdit("name", { kind: "text", value: "Ada", target: target(1) })).toEqual({
      pageIndex: 0, annotIndex: 1, fieldName: "name", kind: "text", text: "Ada",
    });
    expect(draftToEdit("agree", { kind: "checkbox", checked: false, target: target(2) })).toEqual({
      pageIndex: 0, annotIndex: 2, fieldName: "agree", kind: "checkbox", checked: false,
    });
    expect(draftToEdit("color", { kind: "radio", target: target(3, 2) })).toEqual({
      pageIndex: 2, annotIndex: 3, fieldName: "color", kind: "radio",
    });
    expect(draftToEdit("langs", { kind: "choice", selected: [0, 2], target: target(4) })).toEqual({
      pageIndex: 0, annotIndex: 4, fieldName: "langs", kind: "choice", selected: [0, 2],
    });
  });

  it("sends all drafts in one write and keeps a draft edited while the write was in flight", async () => {
    const { drafts, dependencies, controller, flush } = setup();
    const write = deferred();
    dependencies.fill.mockReturnValueOnce(write.promise).mockResolvedValueOnce({ contentGeneration: 6 });
    const onsessionSnapshot = vi.fn();
    drafts.set("name", { kind: "text", value: "Ada", target: target(0) });
    drafts.set("agree", { kind: "checkbox", checked: true, target: target(1) });

    controller.request({ sessionId: "session", contentGeneration: 4, onsessionSnapshot });
    await flush();
    expect(controller.status).toBe("saving");
    expect(dependencies.fill).toHaveBeenCalledTimes(1);
    expect(dependencies.fill.mock.calls[0][0]).toBe("session");
    expect(dependencies.fill.mock.calls[0][1]).toBe(4);
    expect(dependencies.fill.mock.calls[0][2].map((edit) => edit.fieldName)).toEqual(["name", "agree"]);

    // Edited during the write: must survive and be sent next with the new generation.
    drafts.set("name", { kind: "text", value: "Ada Lovelace", target: target(0) });
    controller.request({ sessionId: "session", contentGeneration: 4, onsessionSnapshot });
    await flush();
    write.resolve({ contentGeneration: 5 });
    await flush();

    expect(drafts.has("agree")).toBe(false);
    expect(dependencies.fill).toHaveBeenCalledTimes(2);
    expect(dependencies.fill.mock.calls[1][1]).toBe(5);
    expect(dependencies.fill.mock.calls[1][2]).toEqual([
      { pageIndex: 0, annotIndex: 0, fieldName: "name", kind: "text", text: "Ada Lovelace" },
    ]);
    expect(drafts.count).toBe(0);
    expect(onsessionSnapshot.mock.calls).toEqual([[{ contentGeneration: 5 }], [{ contentGeneration: 6 }]]);
    expect(controller.status).toBe("idle");
  });

  it("asks once before invalidating signatures and keeps drafts when declined", async () => {
    const declined = setup({ signatureCount: 1, confirm: false });
    declined.drafts.set("name", { kind: "text", value: "x", target: target(0) });
    declined.controller.request({ sessionId: "session", contentGeneration: 1 });
    await declined.flush();
    expect(declined.dependencies.confirmSignatureInvalidation).toHaveBeenCalledTimes(1);
    expect(declined.dependencies.fill).not.toHaveBeenCalled();
    expect(declined.controller.status).toBe("error");
    expect(declined.drafts.has("name")).toBe(true);

    const accepted = setup({ signatureCount: 2, confirm: true });
    accepted.dependencies.fill.mockResolvedValue({ contentGeneration: 2 });
    accepted.drafts.set("name", { kind: "text", value: "x", target: target(0) });
    accepted.controller.request({ sessionId: "session", contentGeneration: 1 });
    await accepted.flush();
    accepted.drafts.set("name", { kind: "text", value: "y", target: target(0) });
    accepted.controller.request({ sessionId: "session", contentGeneration: 2 });
    await accepted.flush();
    expect(accepted.dependencies.fill).toHaveBeenCalledTimes(2);
    expect(accepted.dependencies.confirmSignatureInvalidation).toHaveBeenCalledTimes(1);
  });

  it("keeps drafts and reports the error when the write fails", async () => {
    const { drafts, dependencies, controller, flush } = setup();
    dependencies.fill.mockRejectedValue(new Error("'code' alanı en fazla 6 karakter kabul ediyor (MaxLen)"));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    drafts.set("code", { kind: "text", value: "ABCDEFGH", target: target(2) });
    controller.request({ sessionId: "session", contentGeneration: 3 });
    await flush();
    expect(controller.status).toBe("error");
    expect(controller.error).toContain("MaxLen");
    expect(drafts.has("code")).toBe(true);
    errorSpy.mockRestore();
  });

  it("never sends a stale generation older than one it already learned", async () => {
    const { drafts, dependencies, controller, flush } = setup();
    dependencies.fill.mockResolvedValue({ contentGeneration: 8 });
    drafts.set("a", { kind: "checkbox", checked: true, target: target(0) });
    controller.request({ sessionId: "session", contentGeneration: 7 });
    await flush();
    drafts.set("b", { kind: "checkbox", checked: true, target: target(1) });
    // The page has not re-rendered yet and still reports generation 7.
    controller.request({ sessionId: "session", contentGeneration: 7 });
    await flush();
    expect(dependencies.fill.mock.calls.map((call) => call[1])).toEqual([7, 8]);
    // An unsigned document is checked once per session.
    expect(dependencies.getFormInfo).toHaveBeenCalledTimes(1);
  });
});
