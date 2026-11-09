import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";

const sessionMocks = vi.hoisted(() => ({ saveSessionOptimizedCopy: vi.fn() }));
const dialogMocks = vi.hoisted(() => ({ save: vi.fn() }));

vi.mock("../services/backend/session.js", () => sessionMocks);
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
const pluginHookMocks = vi.hoisted(() => ({ notifyPluginExportCompleted: vi.fn() }));
vi.mock("../services/app/pluginHookDispatcher.js", () => pluginHookMocks);

import OptimizeModal from "./OptimizeModal.svelte";

const REPORT = {
  inputBytes: 4 * 1024 * 1024,
  outputBytes: 1024 * 1024,
  images: 1,
  imagesResampled: 1,
  imagesRecompressed: 0,
  imageBytesBefore: 3 * 1024 * 1024,
  imageBytesAfter: 256 * 1024,
  unusedXobjects: 1,
  thumbnails: 0,
  alternateImages: 0,
  streamsCompressed: 0,
  signaturesInvalidated: false,
};

describe("OptimizeModal", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    sessionMocks.saveSessionOptimizedCopy.mockReset();
    dialogMocks.save.mockReset();
    pluginHookMocks.notifyPluginExportCompleted.mockReset();
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  it("saves a copy with the chosen preset and shows the report", async () => {
    const onsaved = vi.fn();
    dialogMocks.save.mockResolvedValue("/tmp/rapor_kucultulmus.pdf");
    sessionMocks.saveSessionOptimizedCopy.mockResolvedValue(REPORT);
    render(OptimizeModal, { props: { sessionId: "s", contentGeneration: 3, fileName: "rapor.pdf", onsaved } });

    await fireEvent.change(document.getElementById("optimize-preset"), { target: { value: "screen" } });
    await fireEvent.click(screen.getByRole("button", { name: "Kopyayı kaydet…" }));

    expect(await screen.findByText(/%75 daha küçük/)).toBeTruthy();
    expect(dialogMocks.save).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "rapor_kucultulmus.pdf" }));
    const [sessionId, generation, settings, target] = sessionMocks.saveSessionOptimizedCopy.mock.calls[0];
    expect([sessionId, generation, target]).toEqual(["s", 3, "/tmp/rapor_kucultulmus.pdf"]);
    expect(settings.color).toEqual({ downsample: true, targetPpi: 96, abovePpi: 144, compression: { kind: "jpeg", quality: 80 } });
    expect(screen.getByText("1 kullanılmayan görüntü/form atıldı.")).toBeTruthy();
    expect(onsaved).toHaveBeenCalledWith(expect.stringMatching(/4,0 MB → 1,0 MB/));
  });

  it("reports a saved optimized copy to plugins as pdf with the page count", async () => {
    dialogMocks.save.mockResolvedValue("/tmp/out.pdf");
    sessionMocks.saveSessionOptimizedCopy.mockResolvedValueOnce(REPORT);
    render(OptimizeModal, { props: { sessionId: "s", contentGeneration: 3, fileName: "rapor.pdf", pageCount: 14, onsaved: vi.fn() } });
    await fireEvent.click(screen.getByRole("button", { name: "Kopyayı kaydet…" }));
    expect(await screen.findByText(/%75 daha küçük/)).toBeTruthy();
    expect(pluginHookMocks.notifyPluginExportCompleted).toHaveBeenCalledWith("pdf", 14);
  });

  it("refuses invalid resolutions and switches to custom settings when edited", async () => {
    render(OptimizeModal, { props: { sessionId: "s", contentGeneration: 1 } });
    await fireEvent.input(document.getElementById("optimize-color-above"), { target: { value: "100" } });

    expect(await screen.findByText(/eşik çözünürlüğü/)).toBeTruthy();
    expect(document.getElementById("optimize-preset").value).toBe("custom");
    expect(screen.getByRole("button", { name: "Kopyayı kaydet…" }).disabled).toBe(true);
  });

  it("shows the backend's refusal and keeps the dialog open", async () => {
    dialogMocks.save.mockResolvedValue("/tmp/out.pdf");
    sessionMocks.saveSessionOptimizedCopy.mockRejectedValue(new Error("Parola korumalı belgeler optimize edilemez"));
    const onclose = vi.fn();
    render(OptimizeModal, { props: { sessionId: "s", contentGeneration: 1, onclose } });
    await fireEvent.click(screen.getByRole("button", { name: "Kopyayı kaydet…" }));

    expect((await screen.findByRole("alert")).textContent).toMatch(/Parola korumalı/);
    expect(onclose).not.toHaveBeenCalled();
  });
});
