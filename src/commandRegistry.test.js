import { describe, expect, it, vi } from "vitest";
import {
  COMMANDS,
  filterCommands,
  getCommandById,
  getCommandPresentation,
  moveCommandSelection,
  selectCommand,
  validateCommandFeatureMappings,
} from "./commandRegistry.js";
import { FeatureUnavailableError } from "./featureActionGuard.js";
import { getFeature, isFeatureRunnable } from "./featureRegistry.js";

const productionMac = { build: "production", platform: "macos" };
const developmentMac = { build: "development", platform: "macos" };

describe("Command Palette policy", () => {
  it("maps every command explicitly to a valid feature or null", () => {
    expect(validateCommandFeatureMappings(COMMANDS)).toBe(true);
    expect(COMMANDS.every((command) => Object.hasOwn(command, "featureId"))).toBe(true);
    expect(() => validateCommandFeatureMappings([
      { id: "bad", featureId: "missing_feature" },
    ])).toThrow(/unknown feature/);
    expect(getCommandById("burn-in-pdf")).toBeNull();
  });

  it("filters by query and hides production mock or disabled commands", () => {
    expect(filterCommands("markdown", productionMac)).toEqual([]);
    const ocrCommands = filterCommands("OCR", productionMac);
    expect(ocrCommands.map((command) => command.id)).toContain("run-ocr-page");
  });

  it("maps the sanitization command to the sanitization feature and lists it only while runnable", () => {
    const command = getCommandById("open-sanitization");
    expect(command).toMatchObject({ featureId: "sanitization", hideUnlessRunnable: true });
    expect(command.title).toContain("Temizle");

    const runnable = (context) => getFeature("sanitization").enabled && isFeatureRunnable("sanitization", context);
    for (const context of [productionMac, developmentMac]) {
      const listed = filterCommands("temizle", context).map((entry) => entry.id).includes("open-sanitization");
      expect(listed).toBe(runnable(context));
    }

    // The rule itself, independent of today's registry state: a visible but not runnable
    // feature (development preview) hides the command, a runnable one lists it.
    const runnableProbe = [{ id: "probe-runnable", title: "probe", category: "x", featureId: "search", hideUnlessRunnable: true }];
    expect(filterCommands("", productionMac, runnableProbe).map((entry) => entry.id)).toEqual(["probe-runnable"]);
    const previewProbe = [{ id: "probe-preview", title: "probe", category: "x", featureId: "visual_redaction_mask", hideUnlessRunnable: true }];
    expect(getCommandPresentation(previewProbe[0], developmentMac)).toMatchObject({ visible: true, runnable: false });
    expect(filterCommands("", developmentMac, previewProbe)).toEqual([]);
    expect(filterCommands("", developmentMac, [{ ...previewProbe[0], hideUnlessRunnable: false }])).toHaveLength(1);
  });

  it("wraps keyboard navigation in both directions", () => {
    const commands = filterCommands("", productionMac);
    expect(moveCommandSelection(commands.length - 1, "down", commands)).toBe(0);
    expect(moveCommandSelection(0, "up", commands)).toBe(commands.length - 1);
  });

  it("does not emit select for a disabled developer command", async () => {
    const onSelect = vi.fn();
    const command = getCommandById("camera-scan");
    // Precondition: the mapped feature is still disabled; pick another command if it ships.
    expect(getFeature(command.featureId)).toMatchObject({ status: "disabled", enabled: false });
    expect(getCommandPresentation(command, developmentMac).visible).toBe(true);
    await expect(
      selectCommand(command, developmentMac, onSelect),
    ).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("selects a runnable command", async () => {
    const onSelect = vi.fn().mockReturnValue("selected");
    const command = getCommandById("open-search");
    await expect(selectCommand(command, productionMac, onSelect)).resolves.toBe("selected");
    expect(onSelect).toHaveBeenCalledWith("open-search");
  });

  it("selects the experimental merge-pdf command in production", async () => {
    const onSelect = vi.fn();
    const command = getCommandById("merge-pdf");
    expect(command.featureId).toBe("page_merge_direct");
    expect(getCommandPresentation(command, productionMac)).toMatchObject({ visible: true, runnable: true });
    await selectCommand(command, productionMac, onSelect);
    expect(onSelect).toHaveBeenCalledWith("merge-pdf");
  });

  it("fails closed for unknown command input", async () => {
    const onSelect = vi.fn();
    await expect(selectCommand(null, developmentMac, onSelect)).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
