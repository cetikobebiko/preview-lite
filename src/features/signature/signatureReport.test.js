import { describe, expect, it } from "vitest";
import { normalizeSignatureReport } from "./signatureReport.js";
import { rawReport, rawSignature } from "./signatureReportTestData.js";

describe("normalizeSignatureReport", () => {
  it("keeps every dimension separately and freezes the result", () => {
    const report = normalizeSignatureReport(rawReport());
    expect(report.sessionId).toBe("session-1");
    expect(report.contentGeneration).toBe(3);
    const [signature] = report.signatures;
    expect(signature.integrity).toBe("valid");
    expect(signature.byteRangeStatus).toBe("valid");
    expect(signature.modification).toBe("no_later_revision");
    expect(signature.padesDetected).toBe("pades_b_b");
    expect(signature.signer.commonName).toBe("TEST ONLY RSA Signer");
    expect(Object.isFrozen(report)).toBe(true);
    expect(Object.isFrozen(signature)).toBe(true);
  });

  it("rejects a report that claims trust, revocation or a validated PAdES level", () => {
    for (const overrides of [
      { trust_status: "trusted" },
      { revocation_status: "good" },
      { pades_validated_level: "pades_b_b" },
    ]) {
      expect(() =>
        normalizeSignatureReport(rawReport({ signatures: [rawSignature(overrides)] })),
      ).toThrow(/geçersiz/);
    }
  });

  it("rejects unknown enum values, schema versions and oversized lists", () => {
    expect(() => normalizeSignatureReport(rawReport({ schema_version: 2 }))).toThrow();
    expect(() =>
      normalizeSignatureReport(rawReport({ signatures: [rawSignature({ cryptographic_integrity: "ok" })] })),
    ).toThrow();
    expect(() =>
      normalizeSignatureReport(rawReport({ signatures: Array.from({ length: 257 }, () => rawSignature()) })),
    ).toThrow();
    expect(() => normalizeSignatureReport(null)).toThrow();
  });
});
