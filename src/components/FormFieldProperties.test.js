import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import FormFieldProperties from "./FormFieldProperties.svelte";

afterEach(() => cleanup());

function design(widget) {
  return {
    selection: { sessionId: "s", contentGeneration: 1, pageIndex: 0, widget },
    busy: false,
    error: "",
    updateSelected: vi.fn(async () => null),
    deleteSelected: vi.fn(async () => null),
  };
}

const TEXT = { annot_index: 0, field_name: "form.email", kind: "Text", required: false, read_only: false, multiline: false, options: [] };

describe("FormFieldProperties", () => {
  it("renames with the partial name and sends flags and limits", async () => {
    const store = design(TEXT);
    const { getByLabelText } = render(FormFieldProperties, { props: { design: store } });
    const name = getByLabelText("Alan adı");
    expect(name.value).toBe("email");
    await fireEvent.input(name, { target: { value: "eposta" } });
    await fireEvent.keyDown(name, { key: "Enter" });
    expect(store.updateSelected).toHaveBeenLastCalledWith({ name: "eposta" });

    await fireEvent.click(getByLabelText("Zorunlu"));
    expect(store.updateSelected).toHaveBeenLastCalledWith({ required: true });
    await fireEvent.click(getByLabelText("Çok satırlı"));
    expect(store.updateSelected).toHaveBeenLastCalledWith({ multiline: true });

    const maxLen = getByLabelText("En fazla karakter");
    await fireEvent.input(maxLen, { target: { value: "10" } });
    await fireEvent.blur(maxLen);
    expect(store.updateSelected).toHaveBeenLastCalledWith({ maxLen: 10 });

    const tooltip = getByLabelText("İpucu");
    await fireEvent.blur(tooltip);
    expect(store.updateSelected).toHaveBeenCalledTimes(4);
    await fireEvent.input(tooltip, { target: { value: "E-posta adresiniz" } });
    await fireEvent.blur(tooltip);
    expect(store.updateSelected).toHaveBeenLastCalledWith({ tooltip: "E-posta adresiniz" });

    await fireEvent.click(getByLabelText("Alanı sil"));
    expect(store.deleteSelected).toHaveBeenCalledTimes(1);
  });

  it("shows only the options editor for choice fields", () => {
    const store = design({ ...TEXT, field_name: "country", kind: "ComboBox", options: [{ label: "TR", selected: true }] });
    const { queryByLabelText, getByText } = render(FormFieldProperties, { props: { design: store } });
    expect(queryByLabelText("Çok satırlı")).toBeNull();
    expect(queryByLabelText("En fazla karakter")).toBeNull();
    expect(getByText("Seçenekler")).toBeTruthy();
  });

  it("does not send an unchanged name", async () => {
    const store = design(TEXT);
    const { getByLabelText } = render(FormFieldProperties, { props: { design: store } });
    await fireEvent.blur(getByLabelText("Alan adı"));
    expect(store.updateSelected).not.toHaveBeenCalled();
  });
});
