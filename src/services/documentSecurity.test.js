import { describe, expect, it } from "vitest";
import { ALL_PERMISSIONS, passwordStrength, protectionSettingsError } from "./documentSecurity.js";

const settings = (overrides = {}) => ({
  userPassword: "",
  ownerPassword: "",
  method: "aes256",
  permissions: { ...ALL_PERMISSIONS },
  encryptMetadata: true,
  ...overrides,
});

describe("protection settings", () => {
  it("needs a password, and a distinct permissions password for restrictions", () => {
    expect(protectionSettingsError(settings())).toMatch(/parolası girin/);
    expect(protectionSettingsError(settings({ userPassword: "açık" }))).toBeNull();
    const restricted = { ...ALL_PERMISSIONS, copy: false };
    expect(protectionSettingsError(settings({ userPassword: "a", permissions: restricted }))).toMatch(/izin parolası/);
    expect(protectionSettingsError(settings({ userPassword: "a", ownerPassword: "a", permissions: restricted }))).toMatch(/farklı/);
    expect(protectionSettingsError(settings({ ownerPassword: "sahip", permissions: restricted }))).toBeNull();
  });

  it("limits legacy methods to short ASCII passwords and keeps RC4 metadata encrypted", () => {
    expect(protectionSettingsError(settings({ userPassword: "şifre", method: "aes128" }))).toMatch(/ASCII/);
    expect(protectionSettingsError(settings({ userPassword: "a".repeat(33), method: "rc4_128" }))).toMatch(/32/);
    expect(protectionSettingsError(settings({ userPassword: "ş".repeat(64) }))).toMatch(/127/);
    expect(protectionSettingsError(settings({ userPassword: "x", method: "rc4_128", encryptMetadata: false }))).toMatch(/RC4/);
  });

  it("rates password strength", () => {
    expect(passwordStrength("")).toBe("");
    expect(passwordStrength("abc")).toBe("zayıf");
    expect(passwordStrength("abcdefg1")).toBe("orta");
    expect(passwordStrength("Güvenli-Parola-42")).toBe("güçlü");
  });
});
