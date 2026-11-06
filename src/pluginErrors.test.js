import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { BackendCommandError } from "./services/backend/client.js";
import { FeatureUnavailableError } from "./featureActionGuard.js";
import { getFeaturePresentation } from "./featureRegistry.js";
import {
  PLUGIN_ERROR_MESSAGES,
  describePluginError,
  pluginLifecycleState,
} from "./pluginErrors.js";

const errorSource = readFileSync(
  resolve(process.cwd(), "src-tauri/src/plugins/error.rs"),
  "utf8",
);
const rustWireCodes = [...errorSource.matchAll(/=> "(PLUGIN_[A-Z_]+)"/g)].map((m) => m[1]);

describe("plugin error mapping", () => {
  it("maps every Rust PluginErrorCode wire value to a Turkish message", () => {
    expect(rustWireCodes.length).toBeGreaterThanOrEqual(39);
    const missing = rustWireCodes.filter((code) => !PLUGIN_ERROR_MESSAGES[code]);
    expect(missing).toEqual([]);
    const stale = Object.keys(PLUGIN_ERROR_MESSAGES).filter((code) => !rustWireCodes.includes(code));
    expect(stale).toEqual([]);
  });

  it("covers the codes the plugin UI must explain", () => {
    expect(PLUGIN_ERROR_MESSAGES.PLUGIN_PERMISSION_DENIED).toMatch(/İzin reddedildi/);
    expect(PLUGIN_ERROR_MESSAGES.PLUGIN_RESOURCE_LIMIT_EXCEEDED).toMatch(/kaynak sınırını/);
    expect(PLUGIN_ERROR_MESSAGES.PLUGIN_EXECUTION_TIMEOUT).toMatch(/zaman sınırını/);
    expect(PLUGIN_ERROR_MESSAGES.PLUGIN_PACKAGE_DIGEST_MISMATCH).toMatch(/karantinaya alındı/);
    expect(PLUGIN_ERROR_MESSAGES.PLUGIN_FEATURE_DISABLED).toMatch(/geliştirme derlemesinde/);
  });

  it("reads the code from backend errors, wire errors and plain errors", () => {
    const backendError = new BackendCommandError("startPluginExecution", "raw detail", {
      code: "PLUGIN_HOOK_NOT_DECLARED",
      message: "raw detail",
    });
    expect(describePluginError(backendError)).toBe(
      `${PLUGIN_ERROR_MESSAGES.PLUGIN_HOOK_NOT_DECLARED} (raw detail)`,
    );
    expect(describePluginError({ code: "PLUGIN_EXECUTION_TIMEOUT", message: "" })).toBe(
      PLUGIN_ERROR_MESSAGES.PLUGIN_EXECUTION_TIMEOUT,
    );
    expect(describePluginError(new Error("başka hata"))).toBe("başka hata");
    expect(describePluginError({ code: "UNKNOWN_CODE", message: "ham" })).toBe("ham");
    expect(describePluginError(null)).toBe("Eklenti işlemi başarısız oldu.");
  });

  it("explains a frontend gate refusal with the registry reason", () => {
    const presentation = getFeaturePresentation("plugin_system", { build: "production", platform: "macos" });
    const error = new FeatureUnavailableError("plugin_system", presentation);
    expect(describePluginError(error)).toMatch(/yalnız geliştirme derlemesinde/);
  });
});

describe("plugin lifecycle state", () => {
  it("separates quarantine from a plugin that was never consented", () => {
    expect(pluginLifecycleState({ consent_granted: true, enabled: true })).toBe("enabled");
    expect(pluginLifecycleState({ consent_granted: true, enabled: false })).toBe("disabled");
    expect(pluginLifecycleState({ consent_granted: false, reviewed_package_digest: "" })).toBe("consent_required");
    expect(pluginLifecycleState({ consent_granted: false, reviewed_package_digest: "abc" })).toBe("quarantined");
  });
});
