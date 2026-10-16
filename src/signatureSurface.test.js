import { describe, expect, it } from "vitest";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { CONTRACTS } from "./services/backend/contracts.js";
import * as actions from "./services/actions.js";
import {
  featureRegistry,
  getFeature,
  isFeatureRunnable,
} from "./featureRegistry.js";
import { createUiState } from "./stores/uiState.svelte.js";

const sourceRoot = resolve(import.meta.dirname);
const productionMac = { build: "production", platform: "macos" };
const developmentMac = { build: "development", platform: "macos" };

function getAllSourceFiles(dir, fileList = []) {
  const entries = readdirSync(dir);
  for (const entry of entries) {
    const fullPath = resolve(dir, entry);
    if (entry === "node_modules" || entry === "dist" || entry === ".git") {
      continue;
    }
    const stat = statSync(fullPath);
    if (stat.isDirectory()) {
      getAllSourceFiles(fullPath, fileList);
    } else if (
      (entry.endsWith(".js") || entry.endsWith(".svelte")) &&
      !entry.endsWith(".test.js")
    ) {
      fileList.push(fullPath);
    }
  }
  return fileList;
}

describe("Signature Surface Invariants (Gate 1)", () => {
  it("ensures no signature command contracts exist in contracts.js", () => {
    const forbiddenContracts = [
      "listSystemCertificates",
      "signPdfPades",
      "verifyPdfSignatures",
      "enableLtv",
    ];
    for (const key of forbiddenContracts) {
      expect(CONTRACTS).not.toHaveProperty(key);
    }
    const contractsSource = readFileSync(
      resolve(sourceRoot, "services/backend/contracts.js"),
      "utf8",
    );
    for (const cmd of [
      "list_system_certificates",
      "sign_pdf_pades",
      "verify_pdf_signatures",
      "enable_ltv",
    ]) {
      expect(contractsSource).not.toContain(`"${cmd}"`);
    }
  });

  it("ensures backend signatures wrapper module does not exist", () => {
    expect(
      existsSync(resolve(sourceRoot, "services/backend/signatures.js")),
    ).toBe(false);
  });

  it("ensures no signature actions are exported from actions.js", () => {
    const forbiddenActions = [
      "listSystemCertificatesAction",
      "signPdfPadesAction",
      "verifyPdfSignaturesAction",
      "enableLtvAction",
    ];
    for (const action of forbiddenActions) {
      expect(actions).not.toHaveProperty(action);
    }
  });

  it("ensures all three signature UI components and their test are deleted", () => {
    const deletedFiles = [
      "components/SignatureWizardModal.svelte",
      "components/SignaturePanel.svelte",
      "components/SignatureSettingsModal.svelte",
      "components/SignaturePanel.test.js",
    ];
    for (const file of deletedFiles) {
      expect(existsSync(resolve(sourceRoot, file))).toBe(false);
    }
  });

  it("ensures signature components are not mounted or imported in AppModalHost or the toolbar", () => {
    const appModalHost = readFileSync(
      resolve(sourceRoot, "components/AppModalHost.svelte"),
      "utf8",
    );
    expect(appModalHost).not.toContain("SignaturePanel");
    expect(appModalHost).not.toContain("SignatureWizardModal");
    expect(appModalHost).not.toContain("SignatureSettingsModal");
    expect(appModalHost).not.toContain("signature_wizard");
    expect(appModalHost).not.toContain("signature_settings");
    expect(appModalHost).not.toContain("signature_verification");
    expect(appModalHost).not.toContain("pades_signing");

    for (const file of ["components/AppToolbar.svelte", "services/app/toolbarMenu.js"]) {
      const toolbar = readFileSync(resolve(sourceRoot, file), "utf8");
      expect(toolbar, file).not.toContain("pades_signing");
      expect(toolbar, file).not.toContain("signature_verification");
      expect(toolbar, file).not.toContain("signature_settings");
      expect(toolbar, file).not.toContain("signature_wizard");
    }
  });

  it("ensures uiState does not contain signature panel or modal properties", () => {
    const ui = createUiState();
    expect(ui.panels).not.toHaveProperty("signature");
    expect(ui).not.toHaveProperty("isSignaturePanelOpen");
    expect(ui).not.toHaveProperty("isSignatureWizardOpen");
    expect(ui).not.toHaveProperty("isSignatureSettingsOpen");
  });

  it("ensures all four signature features are disabled and not runnable", () => {
    const signatureFeatureIds = [
      "pades_signing",
      "signature_verification",
      "ltv",
      "signature_settings",
    ];
    for (const id of signatureFeatureIds) {
      const record = getFeature(id);
      expect(record).toBeDefined();
      expect(record.status).toBe("disabled");
      expect(record.enabled).toBe(false);
      expect(isFeatureRunnable(id, productionMac)).toBe(false);
      expect(isFeatureRunnable(id, developmentMac)).toBe(false);
    }
  });

  it("ensures production frontend source code contains no PIN, TSA credentials, or mock root certificates", () => {
    const allSources = getAllSourceFiles(sourceRoot);
    const forbiddenPatterns = [
      /\btsaPassword\b/,
      /\btimestamp\.digicert\b/,
      /DigiCert Assured ID Root CA/,
      /GlobalSign Root CA/,
      /_signed\.pdf/,
    ];

    for (const filePath of allSources) {
      // Exclude PasswordModal which is for PDF document authentication
      if (filePath.endsWith("PasswordModal.svelte")) {
        continue;
      }
      const content = readFileSync(filePath, "utf8");
      for (const pattern of forbiddenPatterns) {
        expect(
          content,
          `File ${filePath} should not contain pattern ${pattern}`,
        ).not.toMatch(pattern);
      }
    }
  });
});
