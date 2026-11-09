import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushSync, mount, unmount } from "svelte";
import { get } from "svelte/store";
import CommandPalette from "../../components/CommandPalette.svelte";
import { installedPlugins, pluginBackendGate } from "../../pluginStore.js";
import { pluginCommandsFor, pluginIdFromCommandId, pluginPaletteCommands } from "./pluginCommands.js";
import { createFeatureRuntime } from "./featureRuntime.js";

const hello = {
  id: "hello-log", name: "Hello Log", hooks: ["on_custom_command", "on_document_open"],
  enabled: true, consent_granted: true,
};
const openGate = { checked: true, enabled: true, buildProfile: "debug" };

describe("plugin palette commands", () => {
  afterEach(() => {
    installedPlugins.set([]);
    pluginBackendGate.set({ checked: false, enabled: false, buildProfile: null });
  });

  it("lists one 'Eklenti: <ad>' command per enabled plugin declaring on_custom_command", () => {
    const commands = pluginCommandsFor([
      hello,
      { ...hello, id: "off", name: "Off", enabled: false },
      { ...hello, id: "noconsent", name: "No Consent", consent_granted: false },
      { ...hello, id: "nohook", name: "No Hook", hooks: ["on_document_open"] },
    ], openGate);
    expect(commands).toEqual([{
      id: "plugin-command:hello-log", title: "Eklenti: Hello Log", icon: "🧩",
      category: "Eklentiler", featureId: "plugin_system", pluginId: "hello-log",
    }]);
    expect(pluginIdFromCommandId(commands[0].id)).toBe("hello-log");
    expect(pluginIdFromCommandId("open-metadata")).toBeNull();
    expect(pluginIdFromCommandId("plugin-command:")).toBeNull();
  });

  it("shows no plugin commands while the backend gate is closed or unknown", () => {
    expect(pluginCommandsFor([hello], { checked: false, enabled: false })).toEqual([]);
    expect(pluginCommandsFor([hello], { checked: true, enabled: false })).toEqual([]);
  });

  it("follows enabling and disabling a plugin", () => {
    pluginBackendGate.set(openGate);
    installedPlugins.set([hello]);
    expect(get(pluginPaletteCommands).map((c) => c.title)).toEqual(["Eklenti: Hello Log"]);
    installedPlugins.set([{ ...hello, enabled: false }]);
    expect(get(pluginPaletteCommands)).toEqual([]);
    installedPlugins.set([hello]);
    expect(get(pluginPaletteCommands)).toHaveLength(1);
  });
});

describe("CommandPalette with plugin commands", () => {
  let component = null;
  const originalScroll = Element.prototype.scrollIntoView;
  beforeEach(() => { Element.prototype.scrollIntoView = vi.fn(); });
  afterEach(async () => {
    Element.prototype.scrollIntoView = originalScroll;
    if (component) await unmount(component);
    component = null;
    document.body.replaceChildren();
  });

  function renderPalette(context, extraCommands) {
    const onselect = vi.fn();
    component = mount(CommandPalette, {
      target: document.body,
      props: { isOpen: true, context, onselect, onclose: vi.fn(), extraCommands },
    });
    flushSync();
    return { onselect };
  }

  const titles = () => [...document.querySelectorAll(".cmd-item")].map((item) => item.textContent);

  it("lists and selects a plugin command in a development build", async () => {
    const { onselect } = renderPalette({ build: "development", platform: "macos" }, pluginCommandsFor([hello], openGate));
    const item = [...document.querySelectorAll(".cmd-item")].find((el) => el.textContent.includes("Eklenti: Hello Log"));
    expect(item).toBeTruthy();
    item.click();
    await vi.waitFor(() => expect(onselect).toHaveBeenCalledWith({ detail: "plugin-command:hello-log" }));
  });

  it("does not list plugin commands in a production build", () => {
    renderPalette({ build: "production", platform: "macos" }, pluginCommandsFor([hello], openGate));
    expect(titles().some((title) => title.includes("Eklenti: Hello Log"))).toBe(false);
  });
});

describe("featureRuntime plugin command routing", () => {
  beforeEach(() => vi.stubEnv("DEV", true));
  afterEach(() => vi.unstubAllEnvs());

  function runtimeWith(pluginHooks, onToast = vi.fn()) {
    const noop = () => ({ start() {}, destroy() {}, reconcileContext() {} });
    return createFeatureRuntime({
      viewer: { activeImageSourcePath: "", currentPageIndex: 0, pageOrder: [0, 1], rotation: 0, flipH: false, flipV: false },
      annotation: { annotationsMap: {} },
      documentSession: { activeSessionId: "s1", contentGeneration: 1 },
      searchState: {}, sensitiveScanState: {}, secureRedactionState: null, sanitizationState: null,
      ui: { openModal: vi.fn(), panels: {} },
      asyncPrompt: { request: vi.fn() },
      viewerRuntime: { navigation: {}, render: vi.fn() },
      documentRuntime: { isPdf: true, activeDisplayPath: "/a.pdf", fileName: "a.pdf", openFile: vi.fn() },
      callbacks: { isMounted: () => true, saveSettings: vi.fn() },
      tick: async () => {},
      onToast,
      onError: vi.fn(),
      createExportController: vi.fn(() => ({})),
      createSearchController: noop,
      createSensitiveDataScanController: noop,
      listenToEvent: vi.fn(async () => () => {}),
      pluginHooks,
    });
  }

  it("runs on_custom_command for the plugin of a palette command and toasts its failure", async () => {
    const pluginHooks = { runCustomCommand: vi.fn(), exportCompleted: vi.fn() };
    const onToast = vi.fn();
    const runtime = runtimeWith(pluginHooks, onToast);
    await expect(runtime.commandRouter.handleEvent({ detail: "plugin-command:hello-log" })).resolves.toEqual({ status: "handled" });
    expect(pluginHooks.runCustomCommand).toHaveBeenCalledWith("hello-log", { onError: expect.any(Function) });
    pluginHooks.runCustomCommand.mock.calls[0][1].onError({ code: "PLUGIN_NOT_ENABLED", message: "x" });
    expect(onToast).toHaveBeenCalledWith(expect.stringMatching(/^Eklenti komutu başarısız: Eklenti devre dışı/));
  });

  it("still routes ordinary commands", async () => {
    const pluginHooks = { runCustomCommand: vi.fn(), exportCompleted: vi.fn() };
    const runtime = runtimeWith(pluginHooks);
    await expect(runtime.commandRouter.dispatch("open-metadata")).resolves.toEqual({ status: "handled" });
    expect(pluginHooks.runCustomCommand).not.toHaveBeenCalled();
  });
});
