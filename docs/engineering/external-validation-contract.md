# External Output Validation & Standards Contract (11I-E5)

## 1. Purpose & Scope

This document specifies the deterministic, fail-closed external validation contract and standards evidence architecture for application-produced PDF outputs.

The contract ensures that application outputs can be independently validated by external standards tools (e.g. VeraPDF) when compliance claims are made, while enforcing strict boundaries between:
- **Internal verification authority** (independent worker processes verifying semantic transforms like secure redaction and sanitization) and **external standards validators** (checking ISO file-format profiles);
- **Product test PASS** and **standards compliance PASS**;
- **Mock test execution** (`is_mock: true`) and **real compliance evidence**.

Execution of real external tools occurs in **11V-E** (exact-artifact smoke) and **11R** (release certification). In strict offline environments where tools are absent, the harness reports `TOOL_UNAVAILABLE_OFFLINE` without network calls or false-green passes.

---

## 2. Bounded Vocabulary

### Tool Availability States (`ToolAvailability`)
```text
AVAILABLE                 - Tool binary located and version satisfies minimum threshold
TOOL_UNAVAILABLE          - Tool binary not found or not executable
TOOL_UNAVAILABLE_OFFLINE  - Tool binary absent in strict offline environment (zero network)
VERSION_UNSUPPORTED       - Tool located but version < minimum threshold (e.g. < 1.30.2)
PROFILE_UNAVAILABLE       - Requested validation profile not supported by tool
EXECUTION_FAILED          - Tool returned non-zero exit code, crashed, or was terminated
TIMEOUT                   - Bounded timeout expired before tool finished execution
PASS                      - Tool finished and machine-readable report confirmed PASS
FAIL                      - Tool finished and machine-readable report confirmed FAIL
```

### Requiredness Model (`Requiredness`)
```text
REQUIRED_FOR_CLAIM   - Check is mandatory for a published compliance claim.
                       Absence or non-PASS -> NonCompliant (release-blocking).
OPTIONAL_DIAGNOSTIC  - Check provides supplementary diagnostic evidence.
                       Absence -> ToolUnavailableOffline / DiagnosticOnly (non-blocking).
```

### Overall Classification (`OverallValidationStatus`)
```text
COMPLIANT        - All RequiredForClaim validators executed and reported PASS.
NON_COMPLIANT    - At least one RequiredForClaim check failed, timed out, or was unavailable.
DIAGNOSTIC_ONLY  - No RequiredForClaim checks configured; diagnostic outcomes recorded.
NOT_EXECUTED     - No validation outcomes produced.
```

---

## 3. Strict Boundary Rules

### Rule 1: External Validator PASS ≠ Product PASS
A standards validator PASS (e.g. VeraPDF PDF/A or PDF/UA profile PASS) proves only that the file conforms to the structural ISO specification. It does **not** prove:
- That sensitive content was eradicated in redaction;
- That active scripts or exploits were neutralized;
- That the document was opened or rendered without crash;
- That visual layout matches user expectation;
- That digital signatures are trusted or valid.

### Rule 2: Product Test PASS ≠ Standards PASS
Conversely, successful opening, rendering, or round-tripping of a PDF in PDFium or the application UI does **not** prove standards compliance (e.g. PDF/A, PDF/UA, PDF/X). Internal tests and external compliance evidence must remain separate.

### Rule 3: Redaction & Sanitization Correctness Authority
- **Secure Redaction**: Correctness authority is the internal, independent verifier worker (`VerifySecureRedaction` / `secure_redaction_verifier_v1`). Standards validators cannot verify that confidential strings or pixels were removed. External checks on redacted files are strictly supplementary syntax diagnostics.
- **Sanitization**: Per `pdf-sanitization-threat-model.md`, sanitization full-rewrite explicitly **disclaims** preservation of PDF/A, PDF/X, or PDF/UA compliance. Verification is handled entirely by `sanitization-verification/1`. Standards validation is not required and preservation is not claimed.

### Rule 4: Structure-Tree & Tagged PDF Boundary
Read-only structural checks (e.g. `/StructTreeRoot`, `/ParentTree`, `/MarkInfo` in `a11y.rs`) assess document accessibility. The application does not currently generate or rewrite structure trees. When tagging/authoring features are implemented, structural evidence models must verify `StructTreeRoot` presence, role mapping, and MCID linkage.

### Rule 5: Signature Boundary (Deferred to 11I-G / 11R)
Cryptographic PDF signing (PAdES B-B/T/LT/LTA), signature discovery, ByteRange digest checking, CMS verification, and trust path validation are distinct from format syntax validation. They are owned by **11I-G** (implementation) and **11R** (release certification) and are not conflated with general standards validation.

---

## 4. VeraPDF Integration Contract

### Profile Registry
Supported source-backed profiles:
- **PDF/UA**: `PDF/UA-1` (ISO 14289-1:2014), `PDF/UA-2` (ISO 14289-2:2024)
- **PDF/A**: `PDF/A-1b`, `PDF/A-2b`, `PDF/A-2u`, `PDF/A-3b`, `PDF/A-3u` (ISO 19005 series)
- **PDF/X**: `PDF/X-1a:2001`, `PDF/X-4`, `PDF/X-4p`, `PDF/X-5n`, `PDF/X-5g`, `PDF/X-6`, `PDF/X-6p`, `PDF/X-6n` (ISO 15930 series)

### Invariants
1. **Minimum Version**: `1.30.2` (pinned baseline from `pades-architecture-design.md`). Versions below `1.30.2` fail closed as `VersionUnsupported`.
2. **Machine-Readable Reports**: Prefers `--format xml`. Parses `<validationResult status="passed|failed">` and individual `<rule>` findings.
3. **Fail-Closed Text Parsing**: Text fallback strictly matches line-level `"Validation result: PASSED"` or `"FAILED"`. Brittle substring matching (such as matching "PASSED" inside file paths or debug messages) is prohibited and returns `Unparseable`.
4. **Tool Identity Traceability**: Every outcome captures `ToolIdentity` (`name`, `version`, `profile_version`, `invocation_class`, `resolution_method`).
5. **Process Safety**:
   - Input path safety enforced via `validate_input_pdf_path`: file must exist, be a regular file, canonicalized, and free of null bytes or shell metacharacters.
   - Bounded stdout/stderr capture (default 10 MB limit) prevents memory exhaustion.
   - Bounded execution timeout (default 300s, configurable per requirement).

---

## 5. False-Green Prevention & Exit Code Semantics

The harness adheres to strict fail-closed rules:

| Condition | Requiredness | Outcome Status | Overall Status | Process Exit |
|---|---|---|---|---|
| Required validator PASS | `RequiredForClaim` | `Pass` | `Compliant` | 0 |
| Required validator FAIL | `RequiredForClaim` | `Fail` | `NonCompliant` | Non-zero |
| Required tool missing offline | `RequiredForClaim` | `ToolUnavailableOffline` | `NonCompliant` | Non-zero |
| Required tool timeout | `RequiredForClaim` | `Timeout` | `NonCompliant` | Non-zero |
| Required tool unparseable | `RequiredForClaim` | `ExecutionFailed` | `NonCompliant` | Non-zero |
| Optional validator PASS | `OptionalDiagnostic` | `Pass` | `DiagnosticOnly` | 0 |
| Optional tool missing offline | `OptionalDiagnostic` | `ToolUnavailableOffline` | `DiagnosticOnly` | 0 |
| Optional tool timeout | `OptionalDiagnostic` | `Timeout` | `DiagnosticOnly` | 0 |

Tool absence **never** produces `Pass`.

---

## 6. Exact Artifact Output Evidence Model

Output evidence conforms to `ExternalValidationEvidence`:
```json
{
  "operation_type": "release_smoke",
  "source_fixture_id": "core-release-smoke",
  "output_sha256": "727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611",
  "output_byte_length": 4520,
  "overall_status": "diagnostic_only",
  "timestamp": "unix:1756989600",
  "outcomes": [
    {
      "validator_id": "verapdf-pdf-ua-1",
      "tool_identity": null,
      "status": "tool_unavailable_offline",
      "profile": "PDF/UA-1",
      "machine_result": null,
      "stdout_truncated": null,
      "stderr_truncated": "Tool not installed offline",
      "duration_ms": 0,
      "error": "Tool not installed offline",
      "is_mock": false
    }
  ]
}
```

---

## 7. 11V-E Handoff Matrix

| Operation | Artifact Class | Source Fixture | Generated Output | Validator | Profile | Requiredness | Expected Evidence |
|---|---|---|---|---|---|---|---|
| Smoke Launch & Render | `MACOS_DMG`, `MACOS_APP_BUNDLE` | `core-release-smoke` | In-memory render | None | N/A | None | `macos-smoke-evidence-*.json` |
| Smoke Launch & Render | `LINUX_DEB` | `core-release-smoke` | In-memory render | None | N/A | None | `linux-smoke-evidence-*.json` |
| Smoke Launch & Render | `WINDOWS_MSI`, `WINDOWS_NSIS` | `core-release-smoke` | In-memory render | None | N/A | None | `windows-smoke-evidence-*.json` |
| PDF/UA Machine Validation | Packaged installed app | `core-release-smoke` | `output-smoke.pdf` | VeraPDF | `PDF/UA-1` | `OptionalDiagnostic` | `externalValidationResults` (`TOOL_UNAVAILABLE_OFFLINE` offline) |
| PDF/UA-2 Machine Validation | Packaged installed app | `core-release-smoke` | `output-smoke.pdf` | VeraPDF | `PDF/UA-2` | `OptionalDiagnostic` | `externalValidationResults` (`TOOL_UNAVAILABLE_OFFLINE` offline) |

---

## 8. 11R Release Platform Certification Handoff

For release platform certification:
1. **Compliance Claims Requiring External Evidence**: None in the current release. PDF/UA machine validation is an optional diagnostic capability.
2. **Mandatory Exact-Artifact Checks**:
   - Package install / extraction succeeds
   - Executable launches from installed bundle
   - Packaged PDFium runtime resolves (`source == "packaged"`)
   - Smoke document opens, page count == 2, ≥3 text tokens found, bounded thumbnail render succeeds
   - Production IPC worker spawn (`--run-worker`)
   - Clean shutdown
3. **Redaction & Sanitization Certification**: Verified strictly through internal workers (`secure_redaction_verifier_v1` with `is_verified: true`, `sanitization-verification/1`). External standards validators are not part of this release-blocking gate.
4. **Release-Blocking Failures**:
   - Any failure or crash of mandatory smoke checks;
   - Any failure of internal redaction verification;
   - Any `RequiredForClaim` validator reporting `Fail`, `Timeout`, `VersionUnsupported`, or `ToolUnavailable` (when active).
   - Tool absence of optional diagnostic validators (e.g. VeraPDF) is deferrable and **does not** block release.
