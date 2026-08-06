import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import {
  enableLtvAction,
  verifyPdfSignaturesAction,
} from "../services/actions.js";
import SignaturePanel from "./SignaturePanel.svelte";

const featureState = vi.hoisted(() => ({
  signature_verification: true,
  ltv: true,
}));

vi.mock("../featureRegistry.js", () => ({
  getRuntimeFeatureContext: vi.fn(() => ({ platform: "macos" })),
  getFeaturePresentation: vi.fn((featureId) => ({
    id: featureId,
    label: featureId,
    warning: `${featureId} disabled`,
    visible: true,
    runnable: featureState[featureId] === true,
  })),
}));

vi.mock("../services/actions.js", () => ({
  enableLtvAction: vi.fn(),
  verifyPdfSignaturesAction: vi.fn(),
}));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function signature(subject, ltvEnabled = true) {
  return {
    id: `signature-${subject}`,
    is_valid: true,
    is_document_modified: false,
    is_certificate_trusted: true,
    is_certificate_expired: false,
    has_timestamp: true,
    ltv_enabled: ltvEnabled,
    subject,
    signing_time: null,
    errors: [],
  };
}

beforeEach(() => {
  featureState.signature_verification = true;
  featureState.ltv = true;
  verifyPdfSignaturesAction.mockReset();
  enableLtvAction.mockReset();
  verifyPdfSignaturesAction.mockResolvedValue([]);
  enableLtvAction.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
});

describe("SignaturePanel session-native actions", () => {
  it("sends the active session and generation in the verification payload", async () => {
    render(SignaturePanel, {
      props: {
        isOpen: true,
        sessionId: "session-signature",
        contentGeneration: 7,
      },
    });

    await waitFor(() => {
      expect(verifyPdfSignaturesAction).toHaveBeenCalledWith(
        "session-signature",
        7,
        { platform: "macos" },
      );
    });
  });

  it("sends a session-native LTV payload and re-verifies the same identity", async () => {
    verifyPdfSignaturesAction.mockResolvedValue([
      signature("Signer", false),
    ]);
    const { getByRole } = render(SignaturePanel, {
      props: {
        isOpen: true,
        sessionId: "session-ltv",
        contentGeneration: 11,
      },
    });

    const button = await waitFor(() =>
      getByRole("button", { name: "LTV unavailable · MOCK" }),
    );
    await fireEvent.click(button);

    await waitFor(() => {
      expect(enableLtvAction).toHaveBeenCalledWith(
        "session-ltv",
        11,
        "signature-Signer",
        { platform: "macos" },
      );
      expect(verifyPdfSignaturesAction).toHaveBeenCalledTimes(2);
    });
  });

  it("keeps mock-disabled verification from reaching the backend action", async () => {
    featureState.signature_verification = false;
    render(SignaturePanel, {
      props: {
        isOpen: true,
        sessionId: "session-disabled",
        contentGeneration: 3,
      },
    });

    await Promise.resolve();
    expect(verifyPdfSignaturesAction).not.toHaveBeenCalled();
    expect(enableLtvAction).not.toHaveBeenCalled();
  });

  it("rejects a stale verification result after session identity changes", async () => {
    const stale = deferred();
    verifyPdfSignaturesAction
      .mockImplementationOnce(() => stale.promise)
      .mockResolvedValueOnce([signature("Current Signer")]);
    const view = render(SignaturePanel, {
      props: {
        isOpen: true,
        sessionId: "old-session",
        contentGeneration: 1,
      },
    });
    await waitFor(() =>
      expect(verifyPdfSignaturesAction).toHaveBeenCalledTimes(1),
    );

    await view.rerender({
      isOpen: true,
      sessionId: "new-session",
      contentGeneration: 2,
    });
    await waitFor(() =>
      expect(verifyPdfSignaturesAction).toHaveBeenCalledTimes(2),
    );
    await waitFor(() =>
      expect(view.getByText("Current Signer")).toBeTruthy(),
    );

    stale.resolve([signature("Stale Signer")]);
    await Promise.resolve();
    await Promise.resolve();
    expect(view.queryByText("Stale Signer")).toBeNull();
    expect(view.getByText("Current Signer")).toBeTruthy();
  });
});
