import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import { normalizeSignatureReport } from "../features/signature/signatureReport.js";
import { rawReport, rawSignature } from "../features/signature/signatureReportTestData.js";

const startSignatureInspectionAction = vi.hoisted(() => vi.fn());
vi.mock("../services/actions.js", () => ({ startSignatureInspectionAction }));

import SignatureVerificationPanel from "./SignatureVerificationPanel.svelte";

function respondWith(raw) {
  startSignatureInspectionAction.mockImplementation(() => ({
    jobId: "job-1",
    promise: Promise.resolve(normalizeSignatureReport(raw)),
    cancel: vi.fn(),
  }));
}

async function settle() {
  for (let index = 0; index < 5; index += 1) {
    await Promise.resolve();
    await tick();
  }
}

describe("SignatureVerificationPanel", () => {
  beforeEach(() => startSignatureInspectionAction.mockReset());

  it("shows each dimension separately and never a single valid mark", async () => {
    respondWith(rawReport({ session_id: "session-1", content_generation: 3 }));
    const { getByText, getAllByTestId, container } = render(SignatureVerificationPanel, {
      props: { sessionId: "session-1", contentGeneration: 3 },
    });
    await fireEvent.click(getByText("İmzaları denetle"));
    await settle();
    expect(startSignatureInspectionAction).toHaveBeenCalledWith("session-1", 3, expect.anything());
    expect(getAllByTestId("signature-card")).toHaveLength(1);
    expect(getByText("İmza kriptografik olarak bütün; imzalayanın güvenilirliği değerlendirilmedi")).toBeTruthy();
    expect(container.textContent).toContain("Güven");
    expect(container.textContent).toContain("Değerlendirilmedi");
    expect(container.textContent).not.toMatch(/✓|✔|Geçerli imza/);
    expect(container.querySelector(".text-green-600, .text-emerald-600, .bg-green-500")).toBeNull();
  });

  it("marks broken signatures and a report that no longer matches the document", async () => {
    respondWith(
      rawReport({
        session_id: "session-1",
        content_generation: 3,
        signatures: [rawSignature({ cryptographic_integrity: "invalid" })],
      }),
    );
    const { getByText, getByTestId, queryByTestId, rerender } = render(SignatureVerificationPanel, {
      props: { sessionId: "session-1", contentGeneration: 3 },
    });
    await fireEvent.click(getByText("İmzaları denetle"));
    await settle();
    expect(getByTestId("signature-integrity").classList.contains("text-destructive")).toBe(true);
    expect(queryByTestId("signature-report-stale")).toBeNull();
    await rerender({ sessionId: "session-1", contentGeneration: 4 });
    await settle();
    expect(getByTestId("signature-report-stale")).toBeTruthy();
  });
});
