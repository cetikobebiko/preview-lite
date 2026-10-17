import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/svelte";
import { fillFormFields, getFormInfo, getPageFormFields } from "../pdfState.js";
import {
  createFormDraftState,
  FORM_DRAFTS_CONTEXT,
} from "../stores/formDraftState.svelte.js";
import FormLayer from "./FormLayer.svelte";

vi.mock("../pdfState.js", () => ({
  getPageFormFields: vi.fn(),
  getFormInfo: vi.fn(async () => ({ form_type: "AcroForm", signature_count: 0 })),
  // Commit testleri dışında yazma hiç bitmez; taslak durumu değişmeden kalır.
  fillFormFields: vi.fn(() => new Promise(() => {})),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(async () => true),
}));

// Commit denetleyicisinin gecikme zamanlayıcıları testten teste sızmasın.
beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  cleanup();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.clearAllMocks();
});

function widget(overrides) {
  return {
    annot_index: 0,
    field_name: "field",
    kind: "Text",
    read_only: false,
    required: false,
    value: null,
    checked: null,
    options: [],
    multiline: false,
    password: false,
    multi_select: false,
    editable: false,
    hidden: false,
    normalized_rect: { x: 0.1, y: 0.1, width: 0.3, height: 0.03 },
    ...overrides,
  };
}

const PAGE_WIDGETS = [
  widget({ annot_index: 0, field_name: "name", value: "Ada" }),
  widget({ annot_index: 1, field_name: "notes", multiline: true }),
  widget({ annot_index: 2, field_name: "locked", value: "fixed", read_only: true, required: true }),
  widget({ annot_index: 3, field_name: "agree", kind: "Checkbox", checked: false }),
  widget({ annot_index: 4, field_name: "color", kind: "RadioButton", value: "Red", checked: true }),
  widget({ annot_index: 5, field_name: "color", kind: "RadioButton", value: "Red", checked: false }),
  widget({
    annot_index: 6,
    field_name: "country",
    kind: "ComboBox",
    value: "TR",
    options: [
      { label: "TR", selected: true },
      { label: "DE", selected: false },
    ],
  }),
  widget({
    annot_index: 7,
    field_name: "langs",
    kind: "ListBox",
    multi_select: true,
    options: [
      { label: "English", selected: true },
      { label: "Deutsch", selected: false },
    ],
  }),
  widget({ annot_index: 8, field_name: "secret", hidden: true }),
  widget({ annot_index: 9, field_name: "submit", kind: "PushButton" }),
  widget({ annot_index: 10, field_name: "norect", normalized_rect: null }),
];

function renderLayer(props = {}, drafts = createFormDraftState()) {
  const view = render(FormLayer, {
    props: { sessionId: "form-session", contentGeneration: 3, pageIndex: 1, ...props },
    context: new Map([[FORM_DRAFTS_CONTEXT, drafts]]),
  });
  return { ...view, drafts };
}

function fieldOf(element) {
  return element.closest(".form-field");
}

describe("FormLayer", () => {
  it("loads the page widgets for the session generation and renders editable kinds", async () => {
    getPageFormFields.mockResolvedValue(PAGE_WIDGETS);
    const { getByLabelText, getAllByLabelText, queryByLabelText, container } = renderLayer();

    await waitFor(() => expect(getByLabelText("name")).toBeTruthy());
    expect(getPageFormFields).toHaveBeenCalledWith("form-session", 3, 1);
    expect(getByLabelText("name").value).toBe("Ada");
    expect(getByLabelText("notes").tagName).toBe("TEXTAREA");
    expect(getByLabelText("locked").disabled).toBe(true);
    expect(getByLabelText("locked").getAttribute("aria-required")).toBe("true");
    expect(getByLabelText("agree").type).toBe("checkbox");
    expect(getAllByLabelText("color").map((radio) => radio.checked)).toEqual([true, false]);
    expect(getByLabelText("country").tagName).toBe("SELECT");
    expect(getByLabelText("langs").multiple).toBe(true);
    for (const skipped of ["secret", "submit", "norect"]) {
      expect(queryByLabelText(skipped)).toBeNull();
    }
    expect(container.querySelectorAll(".form-field")).toHaveLength(8);
    expect(fieldOf(getByLabelText("name")).getAttribute("style")).toContain("left: 10%");
  });

  it("keeps typed text as a draft and drops it when the original value returns", async () => {
    getPageFormFields.mockResolvedValue(PAGE_WIDGETS);
    const { getByLabelText, drafts } = renderLayer();
    const input = await waitFor(() => getByLabelText("name"));

    await fireEvent.input(input, { target: { value: "Ada Lovelace" } });
    expect(drafts.get("name")).toEqual({
      kind: "text",
      value: "Ada Lovelace",
      target: { pageIndex: 1, annotIndex: 0 },
    });
    expect(fieldOf(input).classList.contains("dirty")).toBe(true);

    await fireEvent.input(input, { target: { value: "Ada" } });
    expect(drafts.has("name")).toBe(false);
    expect(fieldOf(input).classList.contains("dirty")).toBe(false);
  });

  it("drafts checkbox, radio and choice changes", async () => {
    getPageFormFields.mockResolvedValue(PAGE_WIDGETS);
    const { getByLabelText, getAllByLabelText, drafts } = renderLayer();
    await waitFor(() => getByLabelText("agree"));

    await fireEvent.click(getByLabelText("agree"));
    expect(drafts.get("agree")).toEqual({
      kind: "checkbox",
      checked: true,
      target: { pageIndex: 1, annotIndex: 3 },
    });

    const [red, blue] = getAllByLabelText("color");
    await fireEvent.click(blue);
    expect(drafts.get("color")).toEqual({ kind: "radio", target: { pageIndex: 1, annotIndex: 5 } });
    expect(blue.checked).toBe(true);
    expect(fieldOf(red).classList.contains("dirty")).toBe(true);
    await fireEvent.click(red);
    expect(drafts.has("color")).toBe(false);

    const country = getByLabelText("country");
    country.options[1].selected = true;
    await fireEvent.change(country);
    expect(drafts.get("country")).toEqual({
      kind: "choice",
      selected: [1],
      target: { pageIndex: 1, annotIndex: 6 },
    });

    const langs = getByLabelText("langs");
    langs.options[1].selected = true;
    await fireEvent.change(langs);
    expect(drafts.get("langs")).toEqual({
      kind: "choice",
      selected: [0, 1],
      target: { pageIndex: 1, annotIndex: 7 },
    });
  });

  it("does not paint widgets returned for an old session", async () => {
    let resolveOld;
    getPageFormFields
      .mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValueOnce([]);
    const { rerender, queryByLabelText } = renderLayer({ sessionId: "old-session" });
    await waitFor(() => expect(getPageFormFields).toHaveBeenCalledTimes(1));

    await rerender({ sessionId: "new-session", contentGeneration: 4 });
    await waitFor(() => expect(getPageFormFields).toHaveBeenCalledTimes(2));
    resolveOld(PAGE_WIDGETS);
    await Promise.resolve();
    await Promise.resolve();

    expect(queryByLabelText("name")).toBeNull();
  });

  it("shares drafts between pages through the context store and clears them on session change", async () => {
    const drafts = createFormDraftState();
    const shared = [widget({ annot_index: 0, field_name: "name", value: "Ada" })];
    getPageFormFields.mockResolvedValue(shared);
    const first = renderLayer({ pageIndex: 0 }, drafts);
    const second = render(FormLayer, {
      props: { sessionId: "form-session", contentGeneration: 3, pageIndex: 4 },
      context: new Map([[FORM_DRAFTS_CONTEXT, drafts]]),
    });
    const pageA = within(first.container);
    const pageB = within(second.container);
    const inputA = await waitFor(() => pageA.getByLabelText("name"));
    const inputB = await waitFor(() => pageB.getByLabelText("name"));

    await fireEvent.input(inputA, { target: { value: "Grace" } });
    expect(inputB.value).toBe("Grace");
    expect(fieldOf(inputB).classList.contains("dirty")).toBe(true);

    await first.rerender({ sessionId: "other-session" });
    await waitFor(() => expect(drafts.count).toBe(0));
  });

  it("writes drafts when a text field loses focus and reports the new session snapshot", async () => {
    getPageFormFields.mockResolvedValue(PAGE_WIDGETS);
    fillFormFields.mockResolvedValueOnce({ contentGeneration: 4 });
    const onsessionSnapshot = vi.fn();
    const { getByLabelText, drafts } = renderLayer({ onsessionSnapshot });
    const input = await waitFor(() => getByLabelText("name"));

    await fireEvent.input(input, { target: { value: "Ada Lovelace" } });
    await fireEvent.blur(input);

    await waitFor(() => expect(onsessionSnapshot).toHaveBeenCalledWith({ contentGeneration: 4 }));
    expect(getFormInfo).toHaveBeenCalledWith("form-session", 3);
    expect(fillFormFields).toHaveBeenCalledWith("form-session", 3, [
      { pageIndex: 1, annotIndex: 0, fieldName: "name", kind: "text", text: "Ada Lovelace" },
    ]);
    expect(drafts.has("name")).toBe(false);
  });

  it("keeps drafts and marks the fields when the write fails", async () => {
    getPageFormFields.mockResolvedValue(PAGE_WIDGETS);
    fillFormFields.mockRejectedValueOnce(new Error("'locked' alanı read-only"));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const { getByLabelText, drafts } = renderLayer();
    const checkbox = await waitFor(() => getByLabelText("agree"));

    await fireEvent.click(checkbox);
    await waitFor(() => expect(fieldOf(checkbox).classList.contains("commit-failed")).toBe(true));
    expect(drafts.get("agree").checked).toBe(true);
    expect(fieldOf(checkbox).getAttribute("title")).toContain("read-only");
    errorSpy.mockRestore();
  });

  it("renders nothing when the backend read is refused", async () => {
    getPageFormFields.mockRejectedValue(new Error("FEATURE_UNAVAILABLE"));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const { container } = renderLayer();
    await waitFor(() => expect(errorSpy).toHaveBeenCalled());
    expect(container.querySelectorAll(".form-field")).toHaveLength(0);
    errorSpy.mockRestore();
  });
});
