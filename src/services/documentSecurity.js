// Mirrors `document_security::validate_request`, so the dialog refuses what the backend would.

export const ENCRYPTION_METHODS = Object.freeze([
  Object.freeze({ value: "aes256", label: "AES 256 bit (Acrobat X ve sonrası)" }),
  Object.freeze({ value: "aes128", label: "AES 128 bit (Acrobat 7 ve sonrası)" }),
  Object.freeze({ value: "rc4_128", label: "RC4 128 bit (eski okuyucular)" }),
]);

export const PRINT_PERMISSIONS = Object.freeze([
  Object.freeze({ value: "high_resolution", label: "Yüksek çözünürlükte" }),
  Object.freeze({ value: "low_resolution", label: "Düşük çözünürlükte" }),
  Object.freeze({ value: "none", label: "İzin verilmez" }),
]);

export const ALL_PERMISSIONS = Object.freeze({
  print: "high_resolution",
  modify: true,
  copy: true,
  annotate: true,
  fillForms: true,
  assemble: true,
});

const LEGACY_MAX_BYTES = 32;
const AES256_MAX_BYTES = 127;

const utf8Length = (text) => new TextEncoder().encode(text).length;

export function isUnrestricted(permissions) {
  return permissions.print === "high_resolution"
    && permissions.modify && permissions.copy && permissions.annotate
    && permissions.fillForms && permissions.assemble;
}

/** The first reason the backend would refuse these settings, or null. */
export function protectionSettingsError(settings) {
  const user = settings.userPassword;
  const owner = settings.ownerPassword;
  if (!user && !owner) return "Açma parolası ya da izin parolası girin.";
  if (!isUnrestricted(settings.permissions)) {
    if (!owner) return "İzinleri kısıtlamak için bir izin parolası girin.";
    if (owner === user) return "İzin parolası açma parolasından farklı olmalı.";
  }
  for (const password of [user, owner]) {
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f\u007f-\u009f]/.test(password)) return "Parola denetim karakteri içeremez.";
    if (settings.method === "aes256") {
      if (utf8Length(password) > AES256_MAX_BYTES) return "AES-256 parolası en fazla 127 bayt olabilir.";
    } else {
      if (!/^[\x20-\x7e]*$/.test(password)) {
        return "Bu şifreleme yönteminde parola yalnız ASCII karakterler içerebilir; Türkçe karakterler için AES-256 seçin.";
      }
      if (password.length > LEGACY_MAX_BYTES) return "Bu şifreleme yönteminde parola en fazla 32 karakter olabilir.";
    }
  }
  if (settings.method === "rc4_128" && !settings.encryptMetadata) return "RC4 şifrelemede meta veri de şifrelenir.";
  return null;
}

/** A rough strength hint for an entered password: "zayıf", "orta" or "güçlü". */
export function passwordStrength(password) {
  if (!password) return "";
  const classes = [/[a-zğüşöçı]/, /[A-ZĞÜŞÖÇİ]/, /[0-9]/, /[^A-Za-z0-9ğüşöçıĞÜŞÖÇİ]/]
    .filter((pattern) => pattern.test(password)).length;
  if (password.length >= 12 && classes >= 3) return "güçlü";
  if (password.length >= 8 && classes >= 2) return "orta";
  return "zayıf";
}
