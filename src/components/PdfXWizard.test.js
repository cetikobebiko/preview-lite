import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("PDF/X production boundary", () => {
  it("does not ship a wizard, frontend action, or backend contract", () => {
    expect(existsSync(resolve(process.cwd(), "src/components/PdfXWizard.svelte"))).toBe(false);
    expect(readFileSync(resolve(process.cwd(), "src/services/actions.js"), "utf8")).not.toMatch(/exportPdfXAction|exportSessionPdfX/);
    expect(readFileSync(resolve(process.cwd(), "src/services/backend/contracts.js"), "utf8")).not.toMatch(/exportSessionPdfX|export_session_pages_as_pdf_x/);
  });
});
