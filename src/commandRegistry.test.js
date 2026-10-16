import { describe, expect, it, vi } from "vitest";
import {
  COMMANDS,
  filterCommands,
  getCommandById,
  moveCommandSelection,
  selectCommand,
  validateCommandFeatureMappings,
} from "./commandRegistry.js";
import { FeatureUnavailableError } from "./featureActionGuard.js";

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

  it("wraps keyboard navigation in both directions", () => {
    const commands = filterCommands("", productionMac);
    expect(moveCommandSelection(commands.length - 1, "down", commands)).toBe(0);
    expect(moveCommandSelection(0, "up", commands)).toBe(commands.length - 1);
  });

  it("does not emit select for a disabled developer command", async () => {
    const onSelect = vi.fn();
    const command = getCommandById("merge-pdf");
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

  it("fails closed for unknown command input", async () => {
    const onSelect = vi.fn();
    await expect(selectCommand(null, developmentMac, onSelect)).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
