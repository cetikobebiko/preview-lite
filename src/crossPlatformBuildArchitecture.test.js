import { readFileSync, readdirSync } from "node:fs";
import { extname, join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repositoryRoot = process.cwd();
const packageJson = JSON.parse(
  readFileSync(resolve(repositoryRoot, "package.json"), "utf8"),
);
const packageLock = JSON.parse(
  readFileSync(resolve(repositoryRoot, "package-lock.json"), "utf8"),
);

const platformPackagePrefixes = [
  "@esbuild/",
  "@napi-rs/lzma-",
  "@rolldown/binding-",
  "@rollup/rollup-",
  "@tailwindcss/oxide-",
  "@tauri-apps/cli-",
  "lightningcss-",
];

function packageNameFromLockPath(lockPath) {
  const marker = "node_modules/";
  const packagePath = lockPath.slice(lockPath.lastIndexOf(marker) + marker.length);
  const segments = packagePath.split("/");
  return packagePath.startsWith("@")
    ? `${segments[0]}/${segments[1]}`
    : segments[0];
}

function isPlatformPackage(packageName) {
  return platformPackagePrefixes.some((prefix) => packageName.startsWith(prefix));
}

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [".js", ".rs", ".svelte", ".ts"].includes(extname(path)) ? [path] : [];
  });
}

describe("cross-platform build architecture", () => {
  it("pins the lockfile-producing package manager and supported Node runtime", () => {
    expect(packageJson.packageManager).toBe("npm@10.9.2");
    expect(packageJson.engines).toEqual({
      node: "^20.19.0 || ^22.12.0 || >=24.0.0",
      npm: "10.9.x",
    });
    expect(packageLock.lockfileVersion).toBe(3);
    expect(packageLock.packages[""].engines).toEqual(packageJson.engines);
  });

  it("keeps platform binaries optional, constrained, and connected to a parent", () => {
    const packages = Object.entries(packageLock.packages);
    const platformPackages = packages.filter(([lockPath]) =>
      lockPath.includes("node_modules/")
      && isPlatformPackage(packageNameFromLockPath(lockPath)),
    );

    expect(platformPackages.length).toBeGreaterThan(0);
    for (const [lockPath, metadata] of platformPackages) {
      const packageName = packageNameFromLockPath(lockPath);
      expect(metadata.extraneous, lockPath).not.toBe(true);
      expect(metadata.optional, lockPath).toBe(true);
      expect(
        Array.isArray(metadata.os) || Array.isArray(metadata.cpu),
        `${lockPath} must declare an OS or CPU constraint`,
      ).toBe(true);

      const hasParent = packages.some(([, parent]) =>
        parent.optionalDependencies?.[packageName] === metadata.version,
      );
      expect(hasParent, `${lockPath} must have an optional parent edge`).toBe(true);
    }
  });

  it("contains no resolver leftovers", () => {
    const extraneous = Object.entries(packageLock.packages)
      .filter(([, metadata]) => metadata.extraneous)
      .map(([lockPath]) => lockPath);

    expect(extraneous).toEqual([]);
  });

  it("keeps npm scripts independent of POSIX and Windows shells", () => {
    const shellSpecific = [
      /(?:^|\s)(?:rm|cp|mv)\s+(?:-[^\s]+\s+)?/,
      /(?:^|\s)(?:bash|sh|cmd(?:\.exe)?|powershell(?:\.exe)?)\s/,
      /(?:^|\s)(?:export\s+)?[A-Za-z_][A-Za-z0-9_]*=/,
      /(?:^|\s)(?:\.\/|\/bin\/)/,
    ];
    const offenders = Object.entries(packageJson.scripts)
      .filter(([, command]) => shellSpecific.some((pattern) => pattern.test(command)))
      .map(([name]) => name);

    expect(offenders).toEqual([]);
  });

  it("contains no developer-specific absolute path in application source", () => {
    const roots = [
      resolve(repositoryRoot, "src"),
      resolve(repositoryRoot, "src-tauri", "src"),
    ];
    const developerPath = /\/Users\/[^/\s]+\/|\/home\/[^/\s]+\/|[A-Za-z]:\\Users\\[^\\\s]+\\/;
    const offenders = roots.flatMap((root) =>
      sourceFiles(root)
        .filter((path) => developerPath.test(readFileSync(path, "utf8")))
        .map((path) => relative(repositoryRoot, path)),
    );

    expect(offenders).toEqual([]);
  });

  it("keeps test helper binaries out of the app bundle", () => {
    // tauri-cli bundles every binary of the package whose required features
    // are enabled, including ones auto-discovered under src/bin.
    const manifest = readFileSync(
      resolve(repositoryRoot, "src-tauri", "Cargo.toml"),
      "utf8",
    );
    const binEntries = manifest
      .split(/^\[\[bin\]\]\s*$/m)
      .slice(1)
      .map((block) => block.split(/^\[/m)[0])
      .map((block) => ({
        path: block.match(/^path\s*=\s*"([^"]+)"/m)?.[1],
        requiredFeatures: [
          ...(block.match(/^required-features\s*=\s*\[([^\]]*)\]/m)?.[1] ?? "")
            .matchAll(/"([^"]+)"/g),
        ].map((match) => match[1]),
      }));
    const defaultFeatures = [
      ...(manifest.match(/^default\s*=\s*\[([^\]]*)\]/m)?.[1] ?? "").matchAll(
        /"([^"]+)"/g,
      ),
    ].map((match) => match[1]);
    const helperSources = readdirSync(resolve(repositoryRoot, "src-tauri", "src", "bin"))
      .filter((name) => name.endsWith(".rs"))
      .map((name) => `src/bin/${name}`);

    const offenders = helperSources.filter((path) => {
      const entry = binEntries.find((bin) => bin.path === path);
      return (
        !entry ||
        entry.requiredFeatures.length === 0 ||
        entry.requiredFeatures.some(
          (feature) =>
            !feature.endsWith("-test-support") || defaultFeatures.includes(feature),
        )
      );
    });

    expect(helperSources.length).toBeGreaterThan(0);
    expect(offenders).toEqual([]);
  });
});
