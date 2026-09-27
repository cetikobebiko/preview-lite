import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { auditSessionAccessibility, getSessionStructureTree } from "../services/backend/session.js";
import A11yPanel from "./A11yPanel.svelte";

vi.mock("../services/backend/session.js", () => ({
  auditSessionAccessibility: vi.fn(),
  getSessionStructureTree: vi.fn(),
}));
vi.mock("./FeatureNotice.svelte", () => ({ default: vi.fn() }));
vi.mock("./TagTreeExplorer.svelte", () => ({ default: vi.fn() }));

const report = (summaryStatus = "findings_detected") => ({
  summaryStatus,
  referenceProfile: "pdf_ua1_reference",
  documentFacts: { effectiveVersion: "1.7", pageCount: 1, hasStructureTree: false, hasMarkInfo: false, hasMarkedEntry: false, marked: false, xmpTitle: "missing" },
  checks: [], warnings: ["This basic audit does not establish PDF/UA conformance or document accessibility"],
  coverage: { implementedCheckIds: [], notImplementedCheckIds: [], incompleteCheckIds: [], limitations: [] },
  humanReview: [], truncated: false,
});

const structure = () => ({ root: null, findings: [], warnings: [], limitations: [], truncated: false });

afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("A11yPanel session reads", () => {
  it("performs one bounded audit/tree fetch pair for a session generation", async () => {
    auditSessionAccessibility.mockResolvedValue(report());
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "a11y-session", contentGeneration: 9 } });

    await waitFor(() => expect(view.getByText("Findings detected")).toBeTruthy());
    expect(auditSessionAccessibility).toHaveBeenCalledWith("a11y-session", 9);
    expect(getSessionStructureTree).toHaveBeenCalledWith("a11y-session", 9);
    expect(view.queryByText(/\/100/)).toBeNull();
    expect(view.queryByText(/Screen Reader Preview/)).toBeNull();
    expect(view.getByText("Basic accessibility audit")).toBeTruthy();
    expect(view.queryByText(/PDF\/UA compliant|PDF\/UA valid/i)).toBeNull();
    expect(view.queryByText(/Save Changes|Auto-Tag/i)).toBeNull();
  });

  it("does not show audit data returned for an old session", async () => {
    let resolveOld;
    auditSessionAccessibility
      .mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValueOnce(report("audit_incomplete"));
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "old-session", contentGeneration: 1 } });
    await waitFor(() => expect(auditSessionAccessibility).toHaveBeenCalledTimes(1));

    await view.rerender({ sessionId: "new-session", contentGeneration: 2 });
    await waitFor(() => expect(view.getByText("Audit incomplete")).toBeTruthy());
    resolveOld(report("no_findings_in_implemented_checks"));
    await Promise.resolve();
    expect(view.queryByText("No findings in implemented checks")).toBeNull();
  });

  it("does not commit an audit after unmount", async () => {
    let resolveAudit;
    auditSessionAccessibility.mockImplementation(() => new Promise((resolve) => { resolveAudit = resolve; }));
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "session", contentGeneration: 1 } });
    await waitFor(() => expect(auditSessionAccessibility).toHaveBeenCalledOnce());
    view.unmount();
    resolveAudit(report());
    await Promise.resolve();
  });

  it("renders summary counts, MarkInfo facts, coverage and scope limitations", async () => {
    const value = report();
    value.checks = [
      { id: "fail", result: "fail" },
      { id: "warn", result: "warning" },
    ];
    value.humanReview = [
      { id: "human.logical-reading-order", title: "Logical reading order", category: "reading_order" },
      { id: "human.color-contrast", title: "Color contrast and color-only information", category: "visual" },
    ];
    value.coverage = {
      implementedCheckIds: ["fail", "warn"],
      notImplementedCheckIds: ["a", "b", "c"],
      incompleteCheckIds: ["warn"],
      categories: [
        { category: "annotations", status: "not_checked" },
        { category: "forms", status: "not_checked" },
        { category: "text", status: "not_checked" },
        { category: "encryption", status: "not_checked" },
      ],
      limitations: ["Unicode mapping is not checked"],
    };
    value.documentFacts = { ...value.documentFacts, hasMarkInfo: true, hasMarkedEntry: true, marked: false };
    auditSessionAccessibility.mockResolvedValue(value);
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "counts", contentGeneration: 1 } });
    await waitFor(() => expect(view.getByText("Findings: 1")).toBeTruthy());
    expect(view.getByText("Warnings: 1")).toBeTruthy();
    expect(view.getByText("Human review: 2")).toBeTruthy();
    expect(view.getByText("Not checked: 3")).toBeTruthy();
    expect(view.getByText("MarkInfo dictionary")).toBeTruthy();
    expect(view.getByText("Marked entry")).toBeTruthy();
    expect(view.getByText("Marked value")).toBeTruthy();
    expect(view.getAllByText(/does not establish PDF\/UA conformance/)).toHaveLength(2);

    await fireEvent.click(view.getByText("Coverage"));
    expect(view.getByText("Incomplete: 1")).toBeTruthy();
    expect(view.getByText(/annotations/)).toBeTruthy();
    expect(view.getByText(/forms/)).toBeTruthy();
    expect(view.getByText(/text/)).toBeTruthy();
    expect(view.getByText(/encryption/)).toBeTruthy();
    expect(view.getByText("Unicode mapping is not checked")).toBeTruthy();

    await fireEvent.click(view.getByText("Review"));
    expect(view.getByText("Logical reading order")).toBeTruthy();
    expect(view.getByText("Color contrast and color-only information")).toBeTruthy();
  });

  it("keeps incomplete results distinct from no-findings success wording", async () => {
    auditSessionAccessibility.mockResolvedValue(report("audit_incomplete"));
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "incomplete", contentGeneration: 1 } });
    await waitFor(() => expect(view.getByText("Audit incomplete")).toBeTruthy());
    expect(view.queryByText(/No issues were found/)).toBeNull();
  });

  it("rejects a late result after a generation-only change", async () => {
    let resolveOld;
    auditSessionAccessibility
      .mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValueOnce(report("audit_incomplete"));
    getSessionStructureTree.mockResolvedValue(structure());
    const view = render(A11yPanel, { props: { sessionId: "same", contentGeneration: 4 } });
    await waitFor(() => expect(auditSessionAccessibility).toHaveBeenCalledOnce());
    await view.rerender({ sessionId: "same", contentGeneration: 5 });
    await waitFor(() => expect(view.getByText("Audit incomplete")).toBeTruthy());
    resolveOld(report("no_findings_in_implemented_checks"));
    await Promise.resolve();
    expect(view.queryByText("No findings in implemented checks")).toBeNull();
  });

  it("rejects late audit and structure results after session close", async () => {
    let resolveAudit;
    let resolveTree;
    auditSessionAccessibility.mockImplementation(() => new Promise((resolve) => { resolveAudit = resolve; }));
    getSessionStructureTree.mockImplementation(() => new Promise((resolve) => { resolveTree = resolve; }));
    const view = render(A11yPanel, { props: { sessionId: "closing", contentGeneration: 1 } });
    await waitFor(() => expect(auditSessionAccessibility).toHaveBeenCalledOnce());
    await view.rerender({ sessionId: "", contentGeneration: 0 });
    resolveAudit(report("no_findings_in_implemented_checks"));
    resolveTree({ ...structure(), root: { id: "late" } });
    await Promise.resolve();
    expect(view.queryByText("No findings in implemented checks")).toBeNull();
    expect(view.getByText("Open a PDF to run the basic audit.")).toBeTruthy();
  });

  it("shows validation failures as operation errors without committing a report", async () => {
    auditSessionAccessibility.mockRejectedValue(new TypeError("report.schemaVersion: unknown schema version"));
    getSessionStructureTree.mockRejectedValue(new TypeError("inspection.root: expected object"));
    const view = render(A11yPanel, { props: { sessionId: "malformed", contentGeneration: 1 } });
    await waitFor(() => expect(view.getByText(/Audit could not be completed/)).toBeTruthy());
    expect(view.getByText(/unknown schema version/)).toBeTruthy();
    expect(view.queryByText("No findings in implemented checks")).toBeNull();
  });

  it("labels structure output as read-only and not reading-order verification", async () => {
    auditSessionAccessibility.mockResolvedValue(report());
    getSessionStructureTree.mockResolvedValue({ ...structure(), root: { id: "root" } });
    const view = render(A11yPanel, { props: { sessionId: "tree", contentGeneration: 1 } });
    await waitFor(() => expect(view.getByText("Findings detected")).toBeTruthy());
    await fireEvent.click(view.getByText("Structure"));
    expect(view.getByText(/Read-only structural inspection/)).toBeTruthy();
    expect(view.getByText(/not a screen-reader preview or reading-order verification/)).toBeTruthy();
  });
});
