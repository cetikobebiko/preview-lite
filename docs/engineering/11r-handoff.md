# 11R Handoff — Redaction & Sanitization Release Platform Certification

**Owner**: 11R (release platform certification for secure-redaction + sanitization)
**Scope**: The **release** certification surface for the redaction/sanitization
feature set. This is separate from `11I-G` (PAdES cryptographic PDF signing — a
**disabled** feature) and from artifact code-signing (notarization/authenticode,
owned by `11I-G`/`11R`).

---

## 1. What this certifies

| Feature | Current state | Correctness authority |
|---------|---------------|----------------------|
| Secure redaction | managed (active) | `ApplySecureRedaction` + independent `VerifySecureRedaction` worker (`secure_redaction_verifier_v1`) |
| Sanitization | **disabled** (`sanitization.status = "disabled"`, `enabled = false`) | internal `sanitization-verification/1` worker (versioned report) |

> Redaction/sanitization **correctness is NOT proven by a generic external
> validator**. It is proven by the internal, independent verification workers
> (`VerifySecureRedaction` / `sanitization-verification/1`) that reopen the
> immutable snapshot and the fixed internal transform artifact, and re-derive the
> plan from the snapshot. VeraPDF is **not** part of this correctness chain.

## 2. Required exact-artifact checks (§22)

For release certification, the **same** exact-artifact smoke checks hold
(`docs/engineering/11v-e-handoff.json` matrix):

- artifact file exists → deploy (mount/extract/install) succeeds
- main executable present → launch succeeds
- runtime resolves `packaged` (no dev override)
- canonical smoke document opens → page count 2 → ≥3 text tokens → bounded render
- real worker spawn via `--run-worker` (`production_ipc`)
- clean shutdown

These are **required** and **release-blocking** for every candidate platform.

## 3. Required external validators (§22)

**None.**

- VeraPDF (PDF/UA-1, PDF/UA-2) is `OptionalDiagnostic`; the machine-validation
  feature is **disabled** (`src-tauri/src/external_validation/types.rs`
  `documented_requirements()` marks every entry `OptionalDiagnostic`, none
  `RequiredForClaim`).
- Absence of VeraPDF → `TOOL_UNAVAILABLE` in `externalValidationResults`; does
  **not** change `overallStatus` and does **not** block release.
- Redaction/sanitization correctness is **not** routed through the external
  validation harness at all; it uses internal verifier evidence (see §6).

## 4. Platform coverage (§18)

| Platform | Arch | Redaction | Sanitization |
|----------|------|-----------|--------------|
| macOS | arm64 (`aarch64-apple-darwin`) | certify | disabled (no activation) |
| Linux | x86_64 (`x86_64-unknown-linux-gnu`) | certify | disabled |
| Windows | x86_64 (`x86_64-pc-windows-msvc`) | certify | disabled |

Sanitization is **not activated on any platform** in this stage; release
certification for sanitization is accordingly `NOT_APPLICABLE` until activation.

## 5. Release-blocking statuses (§19)

Block release (must be non-PASS → block):

| Evidence | Block when |
|----------|-----------|
| `installResult` / `launchResult` | `FAIL`, `NOT_RUN`, `ARTIFACT_UNAVAILABLE` |
| `runtimeResolutionResult` | `FAIL` or `source != packaged` |
| `smokeDocumentResult` | `FAIL`, `NOT_RUN`, or `workerSpawnMethod != production_ipc` |
| `shutdownResult` | `FAIL`, `NOT_RUN` |
| Redaction `VerifySecureRedaction` | `verification failed` / missing / `not_run` (cannot be `is_verified: true`) |
| Sanitization transform | `verification failed` (when feature activated) |

Deferrable (non-blocking, diagnostic):

| Diagnostic | Defer status |
|-----------|--------------|
| VeraPDF PDF/UA-1 / PDF/UA-2 | `TOOL_UNAVAILABLE` / `NOT_RUN` (optional) |
| Full text extraction, search, annotation, form, OCR, export | out of smoke scope (±diagnostic) |
| String/version compatibility proof (pdfium-render vs runtime) | `UNVERIFIED` (see `platform-support.json` `compatibilityAssumption`) |

## 6. Evidence artifacts (§19)

| Artifact | Produced by | Schema |
|----------|-------------|--------|
| `*-smoke-evidence-*.json` | `artifact-smoke-{macos,linux,windows}.mjs` | `artifact-smoke-evidence-model.json` 1.0.0 |
| Redaction transform report + verifier report | `--run-job-worker` `secure_redaction_verifier_v1` | internal (ordered check set) |
| Sanitization inspection + verification reports | `sanitization-verification/1` | internal (versioned) |
| `externalValidationResults` | `external_validation` harness | optional diagnostic only |

All evidence is machine-readable, SHA-256-bound, and keeps no secrets, keys,
certificates, or real user PDFs. Transform/verifier artifacts stay inside the
job directory; no user document is mutated.

## 7. Explicit separations

1. **Artifact code-signing ≠ PDF cryptographic signing.** Artifact
   code-signing (macOS codesign/notarization, Windows Authenticode) is owned by
   `11I-G`/`11R` and is a packaging/release concern. PDF cryptographic PAdES
   signing / signature verification / LTV is owned by `11I-G` and is a **disabled**
   feature — it is **not** part of the exact-artifact smoke gate.
2. **Internal verifier ≠ external validator.** Secure-redaction/sanitization
   correctness evidence comes from the internal verification workers with
   `is_verified: true` gated on the full ordered check set, not from VeraPDF or
   any generic validator.
3. **Validators are NOT required.** No `RequiredForClaim` external validator
   exists; PDF/UA machine validation is a disabled, optional-diagnostic feature.