import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import {
  analyzeSessionColorSpaces,
  calculateSessionInkCoverage,
  renderSessionSeparationChannels,
} from "../services/backend/prepress.js";
import PrepressPanel from "./PrepressPanel.svelte";

vi.mock("../services/backend/prepress.js", () => ({
  analyzeSessionColorSpaces: vi.fn(),
  calculateSessionInkCoverage: vi.fn(),
  renderSessionSeparationChannels: vi.fn(),
}));

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushAsyncWork() {
  await Promise.resolve();
  await tick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function renderReadyPanel(props = {}, events = {}) {
  return render(PrepressPanel, {
    props: {
      sessionId: "session-test-1",
      pageIndex: 2,
      contentGeneration: 7,
      isDocumentReady: true,
      scale: 1.5,
      ...props,
    },
    events,
  });
}

beforeEach(() => {
  analyzeSessionColorSpaces.mockReset();
  calculateSessionInkCoverage.mockReset();
  renderSessionSeparationChannels.mockReset();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("PrepressPanel document contract", () => {
  it("fiziksel page indexini IPC ve event payload'ında kullanır ve 5 event payload'ında (sessionId, contentGeneration, pageIndex) taşır", async () => {
    const separationResult = {
      c_base64: "c",
      m_base64: "m",
      y_base64: "y",
      k_base64: "k",
    };
    renderSessionSeparationChannels.mockResolvedValue(separationResult);
    calculateSessionInkCoverage.mockResolvedValue({ heatmap_png_base64: "heatmap_data" });

    const onSeparationsReady = vi.fn();
    const onHeatmapReady = vi.fn();
    const onChannelsChanged = vi.fn();
    const onGuidesChanged = vi.fn();
    const onApplyBoxes = vi.fn();

    const { getByRole } = renderReadyPanel({}, {
      separationsReady: onSeparationsReady,
      heatmapReady: onHeatmapReady,
      channelsChanged: onChannelsChanged,
      guidesChanged: onGuidesChanged,
      applyBoxes: onApplyBoxes,
    });

    // 1. separationsReady
    await fireEvent.click(getByRole("button", { name: "Kanalları Çıkar" }));
    await waitFor(() => {
      expect(renderSessionSeparationChannels).toHaveBeenCalledWith(
        "session-test-1",
        7,
        2,
        1.5,
      );
      expect(onSeparationsReady).toHaveBeenCalledOnce();
    });

    expect(onSeparationsReady.mock.calls[0][0].detail).toEqual({
      images: separationResult,
      channels: { c: true, m: true, y: true, k: true },
      sessionId: "session-test-1",
      pageIndex: 2,
      contentGeneration: 7,
    });

    // 2. channelsChanged
    const cCheckbox = getByRole("checkbox", { name: "C" });
    await fireEvent.click(cCheckbox);
    expect(onChannelsChanged).toHaveBeenCalledOnce();
    expect(onChannelsChanged.mock.calls[0][0].detail).toMatchObject({
      sessionId: "session-test-1",
      contentGeneration: 7,
      pageIndex: 2,
      channels: { c: false, m: true, y: true, k: true },
    });

    // 3. heatmapReady
    await fireEvent.click(getByRole("button", { name: "Isı Haritası Çıkar" }));
    await waitFor(() => {
      expect(calculateSessionInkCoverage).toHaveBeenCalledWith(
        "session-test-1",
        7,
        2,
        300,
        1.5,
      );
      expect(onHeatmapReady).toHaveBeenCalledOnce();
      expect(onHeatmapReady.mock.calls[0][0].detail).toMatchObject({
        sessionId: "session-test-1",
        contentGeneration: 7,
        pageIndex: 2,
        heatmap: "heatmap_data",
      });
    });

    // 4. guidesChanged
    const bleedInput = getByRole("spinbutton", { name: "Taşırma Payı (Bleed):" });
    await fireEvent.input(bleedInput, { target: { value: "5.0" } });
    expect(onGuidesChanged).toHaveBeenCalledOnce();
    expect(onGuidesChanged.mock.calls[0][0].detail).toMatchObject({
      sessionId: "session-test-1",
      contentGeneration: 7,
      pageIndex: 2,
      boxes: { bleed: 5.0, trim: 0.0 },
    });

    // 5. applyBoxes
    await fireEvent.click(getByRole("button", { name: "Önizleme Kılavuzlarını Güncelle" }));
    expect(onApplyBoxes).toHaveBeenCalledOnce();
    expect(onApplyBoxes.mock.calls[0][0].detail).toMatchObject({
      sessionId: "session-test-1",
      contentGeneration: 7,
      pageIndex: 2,
      boxes: { bleed: 5.0, trim: 0.0 },
    });
  });

  it("pageIndex -1 olduğunda sayfa IPC'lerini çağırmaz", async () => {
    const { getByRole } = renderReadyPanel({ pageIndex: -1 });
    const separationsButton = getByRole("button", { name: "Kanalları Çıkar" });
    const heatmapButton = getByRole("button", { name: "Isı Haritası Çıkar" });

    expect(separationsButton.disabled).toBe(true);
    expect(heatmapButton.disabled).toBe(true);

    await fireEvent.click(separationsButton);
    await fireEvent.click(heatmapButton);
    expect(renderSessionSeparationChannels).not.toHaveBeenCalled();
    expect(calculateSessionInkCoverage).not.toHaveBeenCalled();
  });

  it("App belge hazırlığını doğrulamadığında PDF uzantısı olsa bile IPC çağırmaz", async () => {
    const { getByRole } = renderReadyPanel({ isDocumentReady: false });
    const reportButton = getByRole("button", { name: "Belgeyi Analiz Et" });

    expect(reportButton.disabled).toBe(true);
    await fireEvent.click(reportButton);
    expect(analyzeSessionColorSpaces).not.toHaveBeenCalled();
  });

  it("page değişiminde sayfa sonuçlarını, generation değişiminde belge raporunu temizler", async () => {
    analyzeSessionColorSpaces.mockResolvedValue({
      rgb_count: 1,
      cmyk_count: 2,
      grayscale_count: 3,
      spot_count: 4,
    });
    renderSessionSeparationChannels.mockResolvedValue({
      c_base64: "c",
      m_base64: "m",
      y_base64: "y",
      k_base64: "k",
    });
    calculateSessionInkCoverage.mockResolvedValue({
      heatmap_png_base64: "heatmap",
    });

    const {
      rerender,
      getByRole,
      getByText,
      queryByRole,
      queryByText,
    } = renderReadyPanel();

    await fireEvent.click(getByRole("button", { name: "Belgeyi Analiz Et" }));
    await fireEvent.click(getByRole("button", { name: "Kanalları Çıkar" }));
    await fireEvent.click(getByRole("button", { name: "Isı Haritası Çıkar" }));

    await waitFor(() => {
      expect(getByText("RGB Objeler:").parentElement.textContent).toContain("1");
      expect(getByText(/Önizleme üzerinde aktif kanallar/)).toBeTruthy();
      expect(getByRole("button", { name: "Temizle" })).toBeTruthy();
    });

    await rerender({
      sessionId: "session-test-1",
      pageIndex: 3,
      contentGeneration: 7,
      isDocumentReady: true,
      scale: 1.5,
    });
    await tick();

    expect(queryByText(/Önizleme üzerinde aktif kanallar/)).toBeNull();
    expect(queryByRole("button", { name: "Temizle" })).toBeNull();
    expect(getByText("RGB Objeler:")).toBeTruthy();

    await rerender({
      sessionId: "session-test-1",
      pageIndex: 3,
      contentGeneration: 8,
      isDocumentReady: true,
      scale: 1.5,
    });
    await tick();
    expect(queryByText("RGB Objeler:")).toBeNull();
  });

  it("prop değişiminde loading'i temizler ve eski async sonucu ayrı request kimliğiyle düşürür", async () => {
    const oldRequest = createDeferred();
    const newRequest = createDeferred();
    renderSessionSeparationChannels
      .mockReturnValueOnce(oldRequest.promise)
      .mockReturnValueOnce(newRequest.promise);

    const onSeparationsReady = vi.fn();
    const { rerender, getByRole, queryByText } = renderReadyPanel(
      { pageIndex: 0 },
      { separationsReady: onSeparationsReady },
    );

    await fireEvent.click(getByRole("button", { name: "Kanalları Çıkar" }));
    expect(getByRole("button", { name: "Hazırlanıyor..." })).toBeTruthy();

    await rerender({
      sessionId: "session-test-1",
      pageIndex: 1,
      contentGeneration: 7,
      isDocumentReady: true,
      scale: 1.5,
    });
    await tick();
    expect(getByRole("button", { name: "Kanalları Çıkar" }).disabled).toBe(false);

    await fireEvent.click(getByRole("button", { name: "Kanalları Çıkar" }));
    newRequest.resolve({ c_base64: "new" });

    await waitFor(() => expect(onSeparationsReady).toHaveBeenCalledOnce());
    expect(onSeparationsReady.mock.calls[0][0].detail.pageIndex).toBe(1);

    oldRequest.resolve({ c_base64: "old" });
    await flushAsyncWork();

    expect(onSeparationsReady).toHaveBeenCalledOnce();
    expect(queryByText("Hazırlanıyor...")).toBeNull();
  });

  it("error state'ini belge generation değişiminde temizler", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    analyzeSessionColorSpaces.mockRejectedValue(new Error("color failed"));

    const { rerender, getByRole, getByText, queryByText } = renderReadyPanel();
    await fireEvent.click(getByRole("button", { name: "Belgeyi Analiz Et" }));
    await waitFor(() => expect(getByText("color failed")).toBeTruthy());

    await rerender({
      sessionId: "session-test-1",
      pageIndex: 2,
      contentGeneration: 9,
      isDocumentReady: true,
      scale: 1.5,
    });
    await tick();
    expect(queryByText("color failed")).toBeNull();
  });
});
