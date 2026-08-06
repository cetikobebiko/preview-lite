import { beforeEach, describe, expect, it, vi } from "vitest";
import { FeatureUnavailableError } from "../featureActionGuard.js";
import {
  applyRedactionsAction,
  enableLtvAction,
  exportPdfXAction,
  listSystemCertificatesAction,
  mergePdfsDirectAction,
  performOcrOnPageAction,
  sanitizePdfDocumentAction,
  scanSensitiveDataAction,
  shareExternalFileMacosAction,
  shareFileMacosAction,
  signPdfPadesAction,
  speakTextAction,
  startSensitiveDataScanAction,
  stopSpeakingAction,
  verifyPdfSignaturesAction,
} from "./actions.js";

const backend = vi.hoisted(() => ({
  mergePdfs: vi.fn(),
  applyRedactions: vi.fn(),
  sanitizePdfDocument: vi.fn(),
  scanSensitiveData: vi.fn(),
  startSensitiveDataScan: vi.fn(),
  enableLtv: vi.fn(),
  listSystemCertificates: vi.fn(),
  signPdfPades: vi.fn(),
  verifyPdfSignatures: vi.fn(),
  exportSessionPdfX: vi.fn(),
  performOcrOnPage: vi.fn(),
  shareExternalFileMacos: vi.fn(),
  shareFileMacos: vi.fn(),
  speakText: vi.fn(),
  stopSpeaking: vi.fn(),
}));

vi.mock("./backend/manipulation.js", () => ({
  mergePdfs: backend.mergePdfs,
}));
vi.mock("./backend/redaction.js", () => ({
  applyRedactions: backend.applyRedactions,
  sanitizePdfDocument: backend.sanitizePdfDocument,
  scanSensitiveData: backend.scanSensitiveData,
  startSensitiveDataScan: backend.startSensitiveDataScan,
}));
vi.mock("./backend/signatures.js", () => ({
  enableLtv: backend.enableLtv,
  listSystemCertificates: backend.listSystemCertificates,
  signPdfPades: backend.signPdfPades,
  verifyPdfSignatures: backend.verifyPdfSignatures,
}));
vi.mock("./backend/prepress.js", () => ({
  exportSessionPdfX: backend.exportSessionPdfX,
}));
vi.mock("./backend/platform.js", () => ({
  performOcrOnPage: backend.performOcrOnPage,
  shareExternalFileMacos: backend.shareExternalFileMacos,
  shareFileMacos: backend.shareFileMacos,
  speakText: backend.speakText,
  stopSpeaking: backend.stopSpeaking,
}));

const productionMac = { build: "production", platform: "macos" };

beforeEach(() => {
  vi.clearAllMocks();
});

describe("feature-aware backend actions", () => {
  it.each([
    ["direct merge", () => mergePdfsDirectAction(["/a.pdf"], "/b.pdf", productionMac), backend.mergePdfs],
    ["redaction", () => applyRedactionsAction("sid", 1, "/b.pdf", [], productionMac), backend.applyRedactions],
    ["sanitization", () => sanitizePdfDocumentAction("sid", 1, "/b.pdf", {}, productionMac), backend.sanitizePdfDocument],
    ["PAdES certificate listing", () => listSystemCertificatesAction(productionMac), backend.listSystemCertificates],
    ["PAdES signing", () => signPdfPadesAction("sid", 1, "/b.pdf", "cert", "pin", {}, productionMac), backend.signPdfPades],
    ["signature verification", () => verifyPdfSignaturesAction("sid", 1, productionMac), backend.verifyPdfSignatures],
    ["LTV", () => enableLtvAction("sid", 1, "sig", productionMac), backend.enableLtv],
    ["PDF/X", () => exportPdfXAction("sid", 1, "/b.pdf", "PDF/X-4", "FOGRA39", productionMac), backend.exportSessionPdfX],
  ])("blocks %s before the raw service", async (_label, action, rawService) => {
    await expect(action()).rejects.toBeInstanceOf(FeatureUnavailableError);
    expect(rawService).not.toHaveBeenCalled();
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
