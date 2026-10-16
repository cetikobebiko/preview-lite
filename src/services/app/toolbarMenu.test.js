import { describe, expect, it, vi } from "vitest";
import { badgeLabel, buildToolbarMenu } from "./toolbarMenu.js";

const gates = (states = {}) => (id) => ({ visible: false, runnable: false, warning: "", badge: "", ...states[id] });
const on = (badge = "") => ({ visible: true, runnable: true, warning: "", badge });

function menu(overrides = {}) {
  const ui = { panels: { a11y: false, prepress: false, sensitiveDataScan: false, readingRuler: false }, togglePanel: vi.fn(), openModal: vi.fn() };
  const onFeatureAction = vi.fn((_id, run) => run());
  const options = {
    isPdf: true,
    hasDocument: true,
    viewer: { historyStack: [], forwardStack: [1], isSinglePageMode: false, isBookMode: false, coverMode: false, isSmartDarkMode: false, isSpeaking: false },
    ui,
    featureUi: gates(),
    onFeatureAction,
    actions: {},
    ...overrides,
  };
  const sections = buildToolbarMenu(options);
  const items = sections.flatMap((section) => section.items);
  return { sections, items, find: (id) => items.find((item) => item.id === id), ui: options.ui, onFeatureAction };
}

describe("toolbar overflow menu", () => {
  it("routes the sensitive data scan entry through the feature guard to its panel", () => {
    const { find, ui, onFeatureAction } = menu({ featureUi: gates({ sensitive_data_scan: on() }) });
    const scan = find("sensitive_data_scan");
    expect(scan).toMatchObject({ label: "Hassas veri taraması", checked: false, disabled: false });

    scan.onSelect();
    expect(onFeatureAction).toHaveBeenCalledWith("sensitive_data_scan", expect.any(Function));
    expect(ui.togglePanel).toHaveBeenCalledWith("sensitiveDataScan");
  });

  it("omits invisible features and disables visible ones that cannot run", () => {
    const { find } = menu({
      featureUi: gates({ local_ai: { visible: true, runnable: false, warning: "Ollama yok", badge: "EXPERIMENTAL" } }),
    });
    expect(find("plugin_system")).toBeUndefined();
    expect(find("local_ai")).toMatchObject({ disabled: true, hint: "Ollama yok", badge: "Deneysel" });
  });

  it("shows prepress when either prepress feature is visible but gates it by resource inspection", () => {
    const { find, onFeatureAction } = menu({
      featureUi: gates({ rgb_derived_print_preview: on(), print_resource_inspection: { visible: false, runnable: false } }),
    });
    const prepress = find("print_resource_inspection");
    expect(prepress.disabled).toBe(true);
    prepress.onSelect();
    expect(onFeatureAction).toHaveBeenCalledWith("print_resource_inspection", expect.any(Function));
  });

  it("keeps page and inspection entries to PDFs and only file entries without a document", () => {
    const image = menu({ isPdf: false, featureUi: gates({ sensitive_data_scan: on() }) });
    expect(image.find("single-page")).toBeUndefined();
    expect(image.find("go-back")).toBeUndefined();
    expect(image.find("sensitive_data_scan")).toBeUndefined();
    expect(image.find("smart-dark")).toBeDefined();

    const idle = menu({ isPdf: false, hasDocument: false, featureUi: gates({ command_palette: on() }) });
    expect(idle.sections.map((section) => section.id)).toEqual(["document", "app"]);
  });

  it("reflects navigation history and view state", () => {
    const { find } = menu();
    expect(find("go-back").disabled).toBe(true);
    expect(find("go-forward").disabled).toBe(false);
    expect(find("single-page").checked).toBe(false);
    expect(find("cover-mode")).toBeUndefined();
  });

  it("swaps speech for a stop entry while speaking", () => {
    const onStopSpeaking = vi.fn();
    const { find } = menu({
      viewer: { historyStack: [], forwardStack: [], isSpeaking: true },
      featureUi: gates({ text_to_speech: on() }),
      actions: { onStopSpeaking },
    });
    find("text_to_speech").onSelect();
    expect(onStopSpeaking).toHaveBeenCalledOnce();
  });

  it("labels feature badges in Turkish", () => {
    expect(badgeLabel("EXPERIMENTAL")).toBe("Deneysel");
    expect(badgeLabel("")).toBe("");
    expect(badgeLabel("UNKNOWN")).toBe("UNKNOWN");
  });
});
