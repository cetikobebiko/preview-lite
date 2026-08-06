import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { tick } from "svelte";
import Harness from "./App.sensitiveScanHarness.svelte";

const appSource = readFileSync(resolve(process.cwd(), "src/App.svelte"), "utf8");

describe("App.svelte sensitive scan effect untrack behavior & harness", () => {
  it("imports untrack from svelte and wraps reconcileSensitiveDataContext in untrack inside App.svelte", () => {
    expect(appSource).toMatch(/import\s*\{[^}]*\buntrack\b[^}]*\}\s*from\s*"svelte"/);
    expect(appSource).toMatch(
      /\$effect\(\(\)\s*=>\s*\{[\s\S]*?\buntrack\(\(\)\s*=>\s*\{[\s\S]*?featureRuntime\.reconcileSensitiveDataContext\(\);[\s\S]*?\}\);[\s\S]*?\}\);/,
    );
  });

  it("keeps 'En az bir aktif kalıp seçilmelidir.' error visible across subsequent ticks, clearing only when panel closes", async () => {
    const startScanMock = vi.fn();
    const { component } = render(Harness, { props: { startScanMock } });

    await tick();
    expect(component.sensitiveScanState.error).toBeNull();

    component.sensitiveScanController.scan([]);
    expect(component.sensitiveScanState.error).toBe("En az bir aktif kalıp seçilmelidir.");

    await tick();
    await tick();
    expect(component.sensitiveScanState.error).toBe("En az bir aktif kalıp seçilmelidir.");

    component.ui.setPanel("sensitiveDataScan", false);
    await tick();
    expect(component.sensitiveScanState.error).toBeNull();
  });

  it("clears validation error when document context changes", async () => {
    const startScanMock = vi.fn();
    const { component } = render(Harness, { props: { startScanMock } });

    await tick();
    component.sensitiveScanController.scan([{ label: "Kalıp", regex: "\\d+", active: false }]);
    expect(component.sensitiveScanState.error).toBe("En az bir aktif kalıp seçilmelidir.");

    await tick();
    await tick();
    expect(component.sensitiveScanState.error).toBe("En az bir aktif kalıp seçilmelidir.");

    component.documentSession.closeSession("session-1");
    await tick();
    expect(component.sensitiveScanState.error).toBeNull();
  });

  it("keeps malformed starter handle error visible across ticks, clearing only on panel/context lifecycle change", async () => {
    const startScanMock = vi.fn(() => ({ jobId: "" }));
    const { component } = render(Harness, { props: { startScanMock } });

    await tick();
    component.sensitiveScanController.scan([{ label: "Kalıp", regex: "\\d+", active: true }]);
    expect(component.sensitiveScanState.error).toBe("Scan job başlatılamadı.");

    await tick();
    await tick();
    expect(component.sensitiveScanState.error).toBe("Scan job başlatılamadı.");

    component.ui.setPanel("sensitiveDataScan", false);
    await tick();
    expect(component.sensitiveScanState.error).toBeNull();
  });

  it("keeps synchronous starter failure visible across ticks, clearing only on panel/context lifecycle change", async () => {
    const startScanMock = vi.fn(() => {
      throw new Error("Senkron starter hatası");
    });
    const { component } = render(Harness, { props: { startScanMock } });

    await tick();
    component.sensitiveScanController.scan([{ label: "Kalıp", regex: "\\d+", active: true }]);
    expect(component.sensitiveScanState.error).toBe("Senkron starter hatası");

    await tick();
    await tick();
    expect(component.sensitiveScanState.error).toBe("Senkron starter hatası");

    component.documentRuntime.isPdf = false;
    await tick();
    expect(component.sensitiveScanState.error).toBeNull();
  });
});
