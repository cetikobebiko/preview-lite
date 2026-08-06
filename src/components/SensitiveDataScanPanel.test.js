import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import SensitiveDataScanPanel from "./SensitiveDataScanPanel.svelte";
import { createSensitiveDataScanState } from "../stores/sensitiveDataScanState.svelte.js";

describe("SensitiveDataScanPanel", () => {
  let state;
  let onScanMock;

  const validGeom = {
    coordinateSpace: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    mediaBox: { left: 0, bottom: 0, width: 400, height: 600 },
    cropBox: null,
    effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 },
    intrinsicRotation: 0,
  };

  beforeEach(() => {
    state = createSensitiveDataScanState();
    onScanMock = vi.fn();
  });

  it("renders title 'Hassas veri tespiti' and 5 warning bullet points", () => {
    const { getByText } = render(SensitiveDataScanPanel, {
      props: { scanState: state, onScan: onScanMock },
    });

    expect(getByText("Hassas veri tespiti")).toBeTruthy();
    expect(getByText("• Tarama text extraction ve seçili pattern’lerle sınırlıdır.")).toBeTruthy();
    expect(getByText("• OCR yapılmamış görsel metni bulmayabilir.")).toBeTruthy();
    expect(getByText("• Sonuçlar eksik veya hatalı olabilir.")).toBeTruthy();
    expect(getByText("• Sonuçlar PDF’den içerik kaldırmaz.")).toBeTruthy();
    expect(getByText("• Güvenli redaksiyon bu sürümde kullanılamaz.")).toBeTruthy();
  });

  it("calls onScan callback when 'Belgeyi Tara' button is clicked", async () => {
    const { getByText } = render(SensitiveDataScanPanel, {
      props: { scanState: state, onScan: onScanMock },
    });

    const scanBtn = getByText("Belgeyi Tara");
    await fireEvent.click(scanBtn);

    expect(onScanMock).toHaveBeenCalled();
  });

  it("renders resolved and unresolved schema v2 report items and handles navigation", async () => {
    const onNavigate = vi.fn();
    const { findByText, getAllByText, getByText } = render(SensitiveDataScanPanel, {
      props: { scanState: state, onScan: onScanMock, onNavigate },
    });

    state.beginScan(["TC Kimlik Numarası", "E-Posta"]);
    state.completeScan({
      schemaVersion: 2,
      truncated: false,
      warnings: [],
      items: [
        {
          pageIndex: 2,
          patternIndex: 0,
          textPreview: "12345678901",
          geometryStatus: "resolved",
          pageGeometry: validGeom,
          sourceRectsPdfPoints: [{ x: 10, y: 20, width: 50, height: 10 }],
          warnings: [],
        },
        {
          pageIndex: 4,
          patternIndex: 1,
          textPreview: "test@domain.com",
          geometryStatus: "unresolved",
          pageGeometry: null,
          sourceRectsPdfPoints: [],
          warnings: [{ code: "UNRESOLVED", message: "Geometri yok" }],
        },
      ],
    }, {});
    await tick();

    const item1 = await findByText("12345678901");
    expect(item1).toBeTruthy();

    await fireEvent.click(item1);
    expect(onNavigate).toHaveBeenCalledWith(2);

    expect(getAllByText(/TC Kimlik Numarası/i).length).toBeGreaterThanOrEqual(1);
    expect(getAllByText(/E-Posta/i).length).toBeGreaterThanOrEqual(1);
  });

  it("captured pattern labels in report UI do NOT change if checkboxes are toggled after scan", async () => {
    const { getAllByText, getAllByRole } = render(SensitiveDataScanPanel, {
      props: { scanState: state, onScan: onScanMock },
    });

    state.beginScan(["TC Kimlik Numarası"]);
    state.completeScan({
      schemaVersion: 2,
      truncated: false,
      warnings: [],
      items: [
        {
          pageIndex: 0,
          patternIndex: 0,
          textPreview: "12345678901",
          geometryStatus: "resolved",
          pageGeometry: validGeom,
          sourceRectsPdfPoints: [{ x: 10, y: 20, width: 50, height: 10 }],
          warnings: [],
        },
      ],
    }, {});
    await tick();

    const initialCount = getAllByText(/TC Kimlik Numarası/i).length;
    expect(initialCount).toBe(2);

    // Toggle checkboxes
    const checkboxes = getAllByRole("checkbox");
    await fireEvent.click(checkboxes[0]);
    await tick();

    // Captured pattern label for patternIndex 0 still evaluates to "TC Kimlik Numarası" because state captured it
    expect(getAllByText(/TC Kimlik Numarası/i).length).toBe(initialCount);
  });

  it("does not render bulk redaction buttons or apply callsite", () => {
    const { queryByText } = render(SensitiveDataScanPanel, {
      props: { scanState: state, onScan: onScanMock },
    });

    expect(queryByText("Tümünü redaksiyona ekle")).toBeNull();
    expect(queryByText("Redaksiyonu Uygula")).toBeNull();
  });
});
