// Frontend registry and Rust backend must make the same plugin gate decision for
// each build profile, and the backend gate must have no webview-callable switch.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { getFeature, isFeatureRunnable } from "./featureRegistry.js";
import { CONTRACTS } from "./services/backend/contracts.js";

const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");
const pluginsMod = read("src-tauri/src/plugins/mod.rs");
const libRs = read("src-tauri/src/lib.rs");
const productionPluginsMod = pluginsMod.slice(0, pluginsMod.indexOf("#[cfg(test)]"));

// Backend decision per profile, as compiled: `plugin_feature_gate_for_build(cfg!(debug_assertions))`
// with `tauri dev` = debug_assertions on, `tauri build` = off.
const BACKEND_DEBUG_ASSERTIONS = { development: true, production: false };

describe("plugin gate parity", () => {
  it("derives the backend gate only from debug_assertions", () => {
    expect(pluginsMod).toMatch(
      /pub const fn plugin_feature_gate_for_build\(debug_assertions: bool\) -> bool \{\s*debug_assertions\s*\}/,
    );
    expect(pluginsMod).toMatch(
      /PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD: bool =\s*plugin_feature_gate_for_build\(cfg!\(debug_assertions\)\);/,
    );
    expect(libRs).toMatch(/plugins::PluginState::for_app\(\s*app\.path\(\)\.app_data_dir\(\)\.ok\(\),?\s*\)/);
    expect(libRs).not.toMatch(/PluginState::new\(/);
  });

  it("has no command that opens the backend gate", () => {
    // set_feature_enabled exists for tests only; no production path calls it.
    const calls = productionPluginsMod.match(/set_feature_enabled\(/g) || [];
    expect(calls).toHaveLength(1); // the definition itself
    expect(libRs).not.toMatch(/set_feature_enabled/);
    const commandNames = Object.values(CONTRACTS)
      .map((contract) => contract.command)
      .filter((name) => typeof name === "string" && name.includes("plugin"));
    expect(commandNames).toContain("get_plugin_feature_status");
    expect(commandNames.filter((name) => /feature/.test(name))).toEqual(["get_plugin_feature_status"]);
  });

  it("frontend runnability equals the backend gate in every build profile", () => {
    expect(getFeature("plugin_system").buildProfiles).toEqual(["development"]);
    for (const [build, debugAssertions] of Object.entries(BACKEND_DEBUG_ASSERTIONS)) {
      for (const platform of ["macos", "windows", "linux"]) {
        expect(isFeatureRunnable("plugin_system", { build, platform })).toBe(debugAssertions);
      }
    }
  });
});
