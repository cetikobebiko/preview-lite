# PDF/X Conversion and Validation Design

## Status and boundary

This document is a future architecture proposal, not an implementation claim. The application does not currently convert to PDF/X, validate PDF/X conformance, perform ICC transforms, or publish a PDF/X artifact. The `pdf_x_export` capability remains disabled and no production IPC handler exists.

The normative baseline must be selected per target, not treated as one generic “PDF/X” mode. ISO/TC 130 lists PDF/X-3 in ISO 15930-6, PDF/X-4 and PDF/X-4p in ISO 15930-7, PDF/X-5 in ISO 15930-8, and PDF/X-6/6p/6n in ISO 15930-9. PDF/X-6 is based on PDF 2.0; older targets have different PDF-version and feature rules. Sources: [ISO/TC 130 standards framework](https://committee.iso.org/files/live/sites/tc130/files/Resources/Framework%20for%20standards%202024.pdf), [ISO 15930-9 catalogue entry](https://www.iso.org/standard/77102.html).

ICC profile parsing, transformation, and output-profile acceptance must use a selected ICC specification and an implementation that can enforce it. The current basic-header observation is not validation. Baseline reference: [ICC.1:2022 specification](https://www.color.org/specifications/ICC.1-2022-05.pdf).

## Target profile comparison gate

Before implementation, X2 must define a machine-readable target profile for each supported variant:

| Concern | PDF/X-1a | PDF/X-4 | PDF/X-6 family |
| --- | --- | --- | --- |
| PDF base/version | target-specific older PDF | PDF 1.6 family | PDF 2.0 |
| Transparency | flatten/remediate policy required | permitted subject to target rules | target-specific PDF 2.0 rules |
| Color | CMYK/gray/spot-oriented complete exchange | color-managed workflow | X-6, X-6p, and X-6n differ |
| Output profile | embedded target policy | embedded or profile variant policy | embedded/external-reference/multicolor depends on target |
| Validation | independent normative validator required | independent normative validator required | independent normative validator required |

The table is planning-level only. Acceptance rules must come from licensed normative text and pinned validator profiles, not from this summary or UI copy. A useful non-normative orientation is [PDF/X in a Nutshell](https://pdfa.org/wp-content/uploads/2017/05/PDFX-in-a-Nutshell.pdf).

## Proposed architecture

1. X2 adds read-only independent validation. It consumes an immutable session snapshot in an isolated worker and emits a digest-bound report. It never mutates the active document.
2. X3 adds licensed output-profile asset management. Every ICC asset has a source, license, semantic version, SHA-256 digest, target eligibility, and platform packaging record.
3. X4 permits metadata and OutputIntent remediation only when content is already valid for the chosen printing condition. A profile name is never a substitute for profile bytes or validation.
4. X5 addresses fonts and forbidden features under an explicit target policy.
5. X6 introduces real color conversion through a selected color-management engine.
6. X7 handles transparency and complex graphics, including any full-rewrite requirement.
7. X8 activates conversion only after all target-specific acceptance gates pass.

All phases remain session-native: the main Tauri process owns identity and publication orchestration, while a worker owns parsing, conversion, serialization, and validation. Input is an immutable snapshot. Output stays inside a private job directory until its digest, worker protocol, target profile, validator report, and expected content generation are verified. Publication uses atomic document I/O and never overwrites the source in place.

## Required policy domains

- PDF version and syntax: target-specific header/catalog version, object and stream rules, forbidden external dependencies, encryption rejection, actions, forms, annotations, optional content, and embedded-file policy.
- OutputIntent: required dictionary fields, subtype, output-condition identifier, registry, embedded versus external profile rules, profile digest, and conflict handling.
- ICC: full header and tag-table validation, component/model compatibility, transform usability, rendering intent, black-point behavior, device-link policy, gamut behavior, and profile conformance.
- Source color resolution: DeviceGray/RGB/CMYK defaults, calibrated spaces, ICCBased, Indexed, Pattern, Shading, images, forms, alternate color spaces, Separation, DeviceN/NChannel, tint transforms, and overprint.
- Fonts: actual glyph usage, complete embedding/subsetting, encoding/CMap consistency, glyph widths, and target-specific forbidden constructs.
- Transparency: blend spaces, groups, soft masks, blend modes, flattening policy for targets that require it, and post-transform verification.
- Spot colors: preservation policy, name conflicts, alternate-space validity, tint-transform behavior, and multicolor target constraints.
- Page geometry: effective Media/Crop/Bleed/Trim/Art boxes and target-specific relationships.
- Full rewrite: conversion must assume a controlled complete rewrite may be necessary. Incremental metadata edits cannot establish compliance.

## Color-management engine options

X3/X6 must compare at least:

- a mature native CMM such as Little CMS;
- platform color systems only if behavior can be made equivalent and testable on all supported targets;
- a commercial engine where licensing, reproducibility, and redistribution justify it.

Selection criteria are ICC-version coverage, DeviceN/spot behavior, deterministic transforms, rendering intents, thread/process safety, Rust FFI risk, CVE response, license, binary provenance, and output equivalence on macOS arm64, Linux x64, and Windows x64. No engine is selected by Prompt 10C.

## Independent validator options

The conversion engine cannot validate its own success as the only oracle. X2 must evaluate an independent validator with:

- explicit ISO 15930 target profiles;
- stable machine-readable findings and exit status;
- deterministic version pinning;
- licensing that permits CI and distribution;
- offline execution;
- macOS arm64, Linux x64, and Windows x64 availability;
- a fixture corpus containing conforming and deliberately non-conforming documents.

Validator output is bound to the exact candidate SHA-256 digest. A pass for one digest cannot authorize publication of another.

## Cross-platform packaging decision record

Every ICC profile, CMM, and validator must receive a per-platform record:

| Asset/dependency | License | Runtime model | Version/checksum pin | Packaging | CI | Maintenance |
| --- | --- | --- | --- | --- | --- | --- |
| ICC profiles | redistribution and modification terms | immutable data asset | required | target resource | digest/read smoke | expiry/replacement owner |
| Color-management engine | source/binary license | static, dynamic, or process | required | macOS/Linux/Windows closure | transform corpus | CVE/API owner |
| Independent validator | CLI/library license | isolated process preferred | required | target binary/runtime | positive/negative corpus | ruleset owner |

No macOS-only assumption is acceptable. Activation requires actual runtime tests on all supported targets, not resolver capability or static compilation alone.

## Acceptance gates

- licensed normative requirements are translated into versioned, reviewable policy rules;
- independent validation passes the exact output digest;
- fonts, encryption, forbidden features, OutputIntent, ICC, transparency, spots, and page boxes have target-specific results;
- conversion and validator versions plus profile digests are recorded in the publication receipt;
- original/working session state is unchanged until atomic publication succeeds;
- cancellation produces no publishable partial artifact;
- macOS arm64, Linux x64, and Windows x64 package/runtime smoke tests pass with pinned assets;
- adversarial and malformed fixture suites pass without unbounded memory, traversal, or output.

## Deferred platform work

PDFium Linux/Windows runtime supply, checksums, licenses, Tauri target resources, installers, signing/notarization, and packaged runtime smoke tests remain Prompt 11 work. Future PDF/X assets add their own cross-platform closure and do not remove that prerequisite.
