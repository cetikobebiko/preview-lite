import { beforeEach, describe, expect, it, vi } from "vitest";
import { FeatureUnavailableError } from "../featureActionGuard.js";
import * as actions from "./actions.js";
import {
  applyRedactionsAction,
  performOcrOnPageAction,
  scanSensitiveDataAction,
  shareExternalFileMacosAction,
  shareFileMacosAction,
  speakTextAction,
  startContinuityCameraScanAction,
  startSensitiveDataScanAction,
  stopSpeakingAction,
} from "./actions.js";

const backend = vi.hoisted(() => ({
  applyRedactions: vi.fn(),
  scanSensitiveData: vi.fn(),
  startSensitiveDataScan: vi.fn(),
  startContinuityCameraScan: vi.fn(),
  performOcrOnPage: vi.fn(),
  shareExternalFileMacos: vi.fn(),
  shareFileMacos: vi.fn(),
  speakText: vi.fn(),
  stopSpeaking: vi.fn(),
}));

vi.mock("./backend/redaction.js", () => ({
  applyRedactions: backend.applyRedactions,
  scanSensitiveData: backend.scanSensitiveData,
  startSensitiveDataScan: backend.startSensitiveDataScan,
}));
vi.mock("./backend/platform.js", () => ({
  performOcrOnPage: backend.performOcrOnPage,
  shareExternalFileMacos: backend.shareExternalFileMacos,
  shareFileMacos: backend.shareFileMacos,
  speakText: backend.speakText,
  startContinuityCameraScan: backend.startContinuityCameraScan,
  stopSpeaking: backend.stopSpeaking,
}));

const productionMac = { build: "production", platform: "macos" };

beforeEach(() => {
  vi.clearAllMocks();
});

describe("feature-aware backend actions", () => {
  it.each([
    ["redaction", () => applyRedactionsAction("sid", 1, "/b.pdf", [], productionMac), backend.applyRedactions],
  ])("blocks %s before the raw service", async (_label, action, rawService) => {
    await expect(action()).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(rawService).not.toHaveBeenCalled();
  });

  it("no longer exposes the path-native direct merge action", () => {
    // The palette merge now runs through mergeSessionPdfs (documentTransformController).
    expect(actions).not.toHaveProperty("mergePdfsDirectAction");
  });

  it("allows enabled sensitive-data scanning and returns the job handle", () => {
    const handle = { jobId: "job-1", promise: Promise.resolve({}), cancel: vi.fn() };
    backend.startSensitiveDataScan.mockReturnValueOnce(handle);
    const result = startSensitiveDataScanAction("sid", 4, ["secret"], productionMac);
    expect(result).toBe(handle);
    expect(backend.startSensitiveDataScan).toHaveBeenCalledWith(
      "sid",
      4,
      ["secret"],
    );
  });

  it("allows enabled sensitive-data scanning and preserves arguments", async () => {
    const report = { schemaVersion: 2, items: [], warnings: [], truncated: false };
    const handle = { jobId: "job-1", promise: Promise.resolve(report), cancel: vi.fn() };
    backend.startSensitiveDataScan.mockReturnValueOnce(handle);
    await expect(
      scanSensitiveDataAction("sid", 4, ["secret"], productionMac),
    ).resolves.toEqual(report);
    expect(backend.startSensitiveDataScan).toHaveBeenCalledWith(
      "sid",
      4,
      ["secret"],
    );
  });

  it("starts a Continuity Camera scan only where the macOS feature is runnable", () => {
    const handle = { jobId: "job-1", promise: Promise.resolve({}), cancel: vi.fn() };
    backend.startContinuityCameraScan.mockReturnValueOnce(handle);
    const fit = { width: 595, height: 842 };
    expect(startContinuityCameraScanAction("sid", 4, 2, fit, productionMac)).toBe(handle);
    expect(backend.startContinuityCameraScan).toHaveBeenCalledWith("sid", 4, 2, fit);

    backend.startContinuityCameraScan.mockClear();
    for (const platform of ["windows", "linux"]) {
      expect(() =>
        startContinuityCameraScanAction("sid", 4, 2, fit, { build: "production", platform }),
      ).toThrow(FeatureUnavailableError);
    }
    expect(backend.startContinuityCameraScan).not.toHaveBeenCalled();
  });

  it("throws synchronous FeatureUnavailableError when startSensitiveDataScanAction runs on unavailable feature", () => {
    const disabledContext = { build: "production", platform: "unsupported_os" };
    expect(() =>
      startSensitiveDataScanAction("sid", 1, ["pattern"], disabledContext)
    ).toThrow(FeatureUnavailableError);
    expect(backend.startSensitiveDataScan).not.toHaveBeenCalled();
  });

  it("returns rejected Promise with FeatureUnavailableError when scanSensitiveDataAction runs on unavailable feature", async () => {
    const disabledContext = { build: "production", platform: "unsupported_os" };
    await expect(
      scanSensitiveDataAction("sid", 1, ["pattern"], disabledContext)
    ).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(backend.startSensitiveDataScan).not.toHaveBeenCalled();
  });

  it.each([
    ["speech", () => speakTextAction("hello", "Yelda", productionMac), backend.speakText, ["hello", "Yelda"]],
    ["stop speech", () => stopSpeakingAction(productionMac), backend.stopSpeaking, []],
    ["PDF share", () => shareFileMacosAction("sid", 3, productionMac), backend.shareFileMacos, ["sid", 3]],
    ["external-file share", () => shareExternalFileMacosAction("/a.png", productionMac), backend.shareExternalFileMacos, ["/a.png"]],
    ["Vision OCR", () => performOcrOnPageAction("sid", 3, 2, productionMac), backend.performOcrOnPage, ["sid", 3, 2]],
  ])("allows %s only through the platform guard", async (_label, action, rawService, args) => {
    rawService.mockResolvedValueOnce("ok");
    await expect(action()).resolves.toBe("ok");
    expect(rawService).toHaveBeenCalledWith(...args);
  });

  it("blocks platform actions before the raw service on unsupported platforms", async () => {
    await expect(
      shareFileMacosAction("sid", 1, {
        build: "production",
        platform: "linux",
      }),
    ).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(backend.shareFileMacos).not.toHaveBeenCalled();
  });
});

describe("startSignatureInspectionAction (Prompt 11 G2)", () => {
  it("is guarded by the signature_verification feature", async () => {
    const { startSignatureInspectionAction } = await import("./actions.js");
    const { FeatureUnavailableError } = await import("../featureActionGuard.js");
    expect(() =>
      startSignatureInspectionAction("s1", 1, { build: "production", platform: "unknown" }),
    ).toThrow(FeatureUnavailableError);
  });
});
