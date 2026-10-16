import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import WorkspacePanels from "./WorkspacePanels.svelte";
import { createSensitiveDataScanState } from "../stores/sensitiveDataScanState.svelte.js";

describe("WorkspacePanels physical->display navigation", () => {
  const validGeom = {
    coordinateSpace: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    mediaBox: { left: 0, bottom: 0, width: 400, height: 600 },
    cropBox: null,
    effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 },
    intrinsicRotation: 0,
  };

  it("calls onJump(0, true) when physicalPageIndex = 2 and pageOrder = [2, 0, 1]", async () => {
    const sensitiveScanState = createSensitiveDataScanState();
    const onJumpMock = vi.fn();

    sensitiveScanState.beginScan(["TC"]);
    sensitiveScanState.completeScan(
      {
        schemaVersion: 2,
        truncated: false,
        warnings: [],
        items: [
          {
            pageIndex: 2,
            patternIndex: 0,
            textPreview: "Physical Page 2 Match",
            geometryStatus: "resolved",
            pageGeometry: validGeom,
            sourceRectsPdfPoints: [{ x: 10, y: 10, width: 10, height: 10 }],
            warnings: [],
          },
        ],
      },
      {},
    );

    const { getByText, getByRole } = render(WorkspacePanels, {
      props: {
        isPdf: true,
        ui: { panels: { sensitiveDataScan: true, sidebar: false, a11y: false, prepress: false }, isModalOpen: () => false, setPanel: vi.fn() },
        viewer: { pageOrder: [2, 0, 1], currentPageIndex: 0, scale: 1 },
        documentSession: { activeSessionId: "session-1", contentGeneration: 1 },
        sensitiveScanState,
        featureUi: () => ({ visible: true }),
        physicalPageIndex: 0,
        isLoadedPdfDocument: true,
        onJump: onJumpMock,
        onScanSensitiveData: vi.fn(),
        onSessionSnapshot: vi.fn(),
        onSeparations: vi.fn(),
        onHeatmap: vi.fn(),
        onChannels: vi.fn(),
        onGuides: vi.fn(),
        onApplyBoxes: vi.fn(),
      },
    });

    expect(getByRole("complementary", { name: "Denetçi" })).toBeTruthy();
    expect(getByRole("heading", { name: "Hassas veri tespiti" })).toBeTruthy();

    const matchPhysical2 = getByText("Physical Page 2 Match");
    await fireEvent.click(matchPhysical2);

    expect(onJumpMock).toHaveBeenCalledWith(0, true);
  });

  it("does NOT call onJump when physicalPageIndex = 1 and pageOrder = [2, 0]", async () => {
    const sensitiveScanState = createSensitiveDataScanState();
    const onJumpMock = vi.fn();

    sensitiveScanState.beginScan(["TC"]);
    sensitiveScanState.completeScan(
      {
        schemaVersion: 2,
        truncated: false,
        warnings: [],
        items: [
          {
            pageIndex: 1,
            patternIndex: 0,
            textPreview: "Physical Page 1 Match",
            geometryStatus: "resolved",
            pageGeometry: validGeom,
            sourceRectsPdfPoints: [{ x: 10, y: 10, width: 10, height: 10 }],
            warnings: [],
          },
        ],
      },
      {},
    );

    const { getByText } = render(WorkspacePanels, {
      props: {
        isPdf: true,
        ui: { panels: { sensitiveDataScan: true, sidebar: false, a11y: false, prepress: false }, isModalOpen: () => false, setPanel: vi.fn() },
        viewer: { pageOrder: [2, 0], currentPageIndex: 0, scale: 1 },
        documentSession: { activeSessionId: "session-1", contentGeneration: 1 },
        sensitiveScanState,
        featureUi: () => ({ visible: true }),
        physicalPageIndex: 0,
        isLoadedPdfDocument: true,
        onJump: onJumpMock,
        onScanSensitiveData: vi.fn(),
        onSessionSnapshot: vi.fn(),
        onSeparations: vi.fn(),
        onHeatmap: vi.fn(),
        onChannels: vi.fn(),
        onGuides: vi.fn(),
        onApplyBoxes: vi.fn(),
      },
    });

    const matchPhysical1 = getByText("Physical Page 1 Match");
    await fireEvent.click(matchPhysical1);

    expect(onJumpMock).not.toHaveBeenCalled();
  });
});
