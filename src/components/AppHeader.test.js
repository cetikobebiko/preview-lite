import { afterEach, describe, expect, it, vi } from "vitest";
import { flushSync, mount, unmount } from "svelte";
import AppHeader from "./AppHeader.svelte";

describe("AppHeader shell smoke", () => {
  let component = null;

  afterEach(async () => {
    if (component) await unmount(component);
    component = null;
    document.body.replaceChildren();
  });

  it("mounts and routes panel changes through the typed ui API", () => {
    const ui = {
      panels: { sidebar: true, a11y: false, prepress: false },
      togglePanel: vi.fn(),
      isModalOpen: vi.fn(() => false),
      openModal: vi.fn(),
    };
    component = mount(AppHeader, {
      target: document.body,
      props: {
        isPdf: true,
        viewer: { activeImageSourcePath: "", fileExt: "pdf" },
        ui,
        fileName: "document.pdf",
        activePdfDisplayPath: "/tmp/document.pdf",
        featureUi: () => ({ visible: false, runnable: false, warning: "" }),
        featureControlLabel: (_id, label) => label,
        onFeatureAction: vi.fn(),
        onSelectFile: vi.fn(),
      },
    });
    flushSync();

    const sidebar = [...document.querySelectorAll("button")]
      .find((button) => button.textContent.includes("SIDEBAR"));
    sidebar.click();
    expect(ui.togglePanel).toHaveBeenCalledWith("sidebar");
  });

  it("routes sensitive_data_scan SCAN button to toggle sensitiveDataScan panel", () => {
    const ui = {
      panels: { sidebar: true, a11y: false, prepress: false, sensitiveDataScan: false },
      togglePanel: vi.fn(),
      isModalOpen: vi.fn(() => false),
      openModal: vi.fn(),
    };
    const onFeatureAction = vi.fn((_id, callback) => callback());

    component = mount(AppHeader, {
      target: document.body,
      props: {
        isPdf: true,
        viewer: { activeImageSourcePath: "", fileExt: "pdf" },
        ui,
        fileName: "document.pdf",
        activePdfDisplayPath: "/tmp/document.pdf",
        featureUi: (id) => ({ visible: id === "sensitive_data_scan", runnable: true, warning: "" }),
        featureControlLabel: (_id, label) => label,
        onFeatureAction,
        onSelectFile: vi.fn(),
      },
    });
    flushSync();

    const scanButton = [...document.querySelectorAll("button")]
      .find((button) => button.textContent.includes("SCAN"));
    expect(scanButton).toBeTruthy();
    scanButton.click();
    expect(onFeatureAction).toHaveBeenCalledWith("sensitive_data_scan", expect.any(Function));
    expect(ui.togglePanel).toHaveBeenCalledWith("sensitiveDataScan");
  });
});
