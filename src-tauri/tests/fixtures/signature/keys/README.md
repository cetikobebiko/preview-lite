# TEST ONLY keys

These RSA private keys exist only to sign the G2 signature verification fixtures
(`../manifest.json`, generator `src-tauri/tests/signature_support/mod.rs`). They protect
nothing and must never be used outside tests.

- Generated once on 2026-11-24 with OpenSSL 4.1.0 (Homebrew):
  `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:<2048|1024> -outform DER`
  then `openssl rsa -inform DER -traditional -outform DER` (PKCS#1).
- EC keys are not stored: the generator derives their scalars from fixed labels.
- Production code never loads private keys (`src-tauri/tests/signature_surface.rs`
  `production_signature_code_uses_no_private_key_or_signing_api`).
