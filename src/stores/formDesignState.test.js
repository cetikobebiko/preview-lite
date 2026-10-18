import { describe, expect, it, vi } from "vitest";
import { createFormDesignState, isFormTool } from "./formDesignState.svelte.js";
import { SignatureLossDeclinedError } from "../services/signatureLossGuard.js";

vi.mock("../pdfState.js", () => ({ authorFormFields: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("../services/backend/session.js", () => ({ getSessionFormInfo: vi.fn() }));

function setup() {
  const author = vi.fn(async () => ({ contentGeneration: 8 }));
  const ensureSigned = vi.fn(async () => {});
  const design = createFormDesignState({ author, ensureSigned });
  const onsessionSnapshot = vi.fn();
  const widget = { annot_index: 3, field_name: "Email", kind: "Text" };
  const select = () =>
    design.select({ sessionId: "s", contentGeneration: 7, pageIndex: 1, widget, onsessionSnapshot });
  return { design, author, ensureSigned, onsessionSnapshot, select };
}

describe("form design state", () => {
  it("recognizes only form tools", () => {
    expect(isFormTool("form-text")).toBe(true);
    expect(isFormTool("pointer")).toBe(false);
    expect(isFormTool("highlight")).toBe(false);
  });

  it("adds a field, reports the snapshot and asks the page to select the newest widget", async () => {
    const { design, author, ensureSigned, onsessionSnapshot } = setup();
    const op = { op: "add", pageIndex: 0, normalizedRect: { x: 0.1, y: 0.1, width: 0.2, height: 0.03 }, field: { kind: "checkbox" }, name: null };
    await design.add({ sessionId: "s", contentGeneration: 7, onsessionSnapshot }, op);
    expect(ensureSigned).toHaveBeenCalledWith("s", 7, "Form alanı değişikliği");
    expect(author).toHaveBeenCalledWith("s", 7, [op]);
    expect(onsessionSnapshot).toHaveBeenCalledWith({ contentGeneration: 8 });
    expect(design.pendingSelect).toEqual({ sessionId: "s", pageIndex: 0, newest: true });
  });

  it("builds move, set_props and delete ops for the selected widget", async () => {
    const { design, author, select } = setup();
    select();
    const rect = { x: 0.2, y: 0.2, width: 0.3, height: 0.04 };
    await design.moveSelected(rect);
    expect(author).toHaveBeenLastCalledWith("s", 7, [
      { op: "move", pageIndex: 1, annotIndex: 3, fieldName: "Email", normalizedRect: rect },
    ]);
    expect(design.pendingSelect).toEqual({ sessionId: "s", pageIndex: 1, fieldName: "Email" });
    expect(design.selection).toBeNull();

    select();
    await design.updateSelected({ name: "Eposta", required: true });
    expect(author).toHaveBeenLastCalledWith("s", 7, [
      { op: "set_props", pageIndex: 1, annotIndex: 3, fieldName: "Email", props: { name: "Eposta", required: true } },
    ]);
    expect(design.pendingSelect.fieldName).toBe("Eposta");

    select();
    await design.deleteSelected();
    expect(author).toHaveBeenLastCalledWith("s", 7, [
      { op: "delete", pageIndex: 1, annotIndex: 3, fieldName: "Email" },
    ]);
    expect(design.pendingSelect).toBeNull();
  });

  it("keeps the selection and reports the error when the write fails", async () => {
    const { design, author, select } = setup();
    author.mockRejectedValueOnce(new Error("'Email' adında bir alan zaten var"));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    select();
    await design.updateSelected({ name: "Email2" });
    expect(design.error).toContain("zaten var");
    expect(design.selection.widget.field_name).toBe("Email");
    expect(design.busy).toBe(false);
    errorSpy.mockRestore();
  });

  it("treats a declined signature warning as a silent cancel", async () => {
    const { design, author, ensureSigned, select } = setup();
    ensureSigned.mockRejectedValueOnce(new SignatureLossDeclinedError());
    select();
    await design.deleteSelected();
    expect(author).not.toHaveBeenCalled();
    expect(design.error).toBe("");
  });
});
