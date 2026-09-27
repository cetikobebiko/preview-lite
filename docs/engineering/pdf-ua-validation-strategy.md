# PDF/UA Validation Strategy

## Capability boundary

The built-in Basic Accessibility Audit is not a PDF/UA validator. Full machine validation must be a separately named, disabled capability (`pdf_ua_machine_validation`) with a separate result model and dependency lifecycle. Human checks must remain separate even after machine validation succeeds.

## Profile selection

- PDF/UA-1: ISO 14289-1:2014, based on PDF 1.7.
- PDF/UA-2: ISO 14289-2:2024, based on PDF 2.0 and including namespace-era semantics.
- Header version, Catalog `/Version`, XMP part/revision and contradictions are all inputs. The basic audit selects the UA-1 reference only for effective PDF 1.7 and UA-2 only for effective PDF 2.0 with the supported revision declaration; mismatches remain undetermined.
- A declaration selects or informs a validation profile but never proves conformance.
- Unknown revisions, conflicting declarations and unsupported PDF versions fail closed.

## Validator candidates

| Candidate | UA profiles and output | Packaging/runtime | Decision |
|---|---|---|---|
| veraPDF | Current releases expose UA-1/UA-2 profiles and CLI structured reports | Java runtime; dual GPLv3+/MPLv2 licensing; JVM, bundle size, CVEs and version pinning require review | Preferred research candidate, not integrated |
| PAC 2026 | PDF/UA/WCAG desktop inspection with human-review tooling | Windows/.NET; no public CLI/batch and no macOS production version | Manual cross-check only |
| axesSense | Commercial validation/service options | Contract, network/privacy, offline and profile-version guarantees require vendor confirmation | Enterprise evaluation only |
| Acrobat Preflight | Commercial desktop validation workflows | No verified redistributable headless structured API for this application | Not an embedded candidate |

An integration must use an immutable session snapshot rather than the active display path, work offline by default, have memory/time/process limits, protect encrypted-document credentials, pin the validator and ruleset version, and pass license/security review. No validator executable or runtime is added by Prompt 10B.

## Separate result model

A future result should be shaped independently from the basic audit:

```text
ExternalPdfUaValidationReport {
  validatorName,
  validatorVersion,
  profile,
  machineValidationResult,
  failedRules,
  reportDigest,
  humanChecksOutstanding,
  warnings
}
```

Machine success may be described only as “PDF/UA machine checks passed.” It must not produce a full conformance statement while required human checks remain.

## Human-review record design

A future `AccessibilityHumanReview` may bind the audit version and document digest to reviewer identity, timestamp, item decisions and notes. Reviewer authentication, signed attestation, persistence, certificate generation and compliance claims are not part of the Basic Accessibility Audit.

## Remaining blockers

- Complete PDF/UA-2 namespace, associated-files and MathML rules
- Full ParentTree/MCID/content-stream and Form XObject validation
- Unicode, ToUnicode and glyph-semantic validation
- Reliable annotation/form identity correlation
- Validator/runtime packaging, credential transport, licensing and update policy
- A recorded and reviewable human-assessment workflow

## Sources

- [ISO 14289-1:2014](https://www.iso.org/standard/64599.html)
- [ISO 14289-2:2024](https://www.iso.org/standard/82278.html)
- [Matterhorn Protocol 1.1](https://pdfa.org/resource/the-matterhorn-protocol/)
- [veraPDF validation documentation](https://docs.verapdf.org/validation/)
- [veraPDF CLI profiles and reports](https://docs.verapdf.org/cli/validation/)
- [PAC download and platform support](https://pac.pdf-accessibility.org/en/download)
- [PAC FAQ](https://pac.pdf-accessibility.org/en/resources/faq)
