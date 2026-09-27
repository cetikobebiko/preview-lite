# Basic Accessibility Audit

## Purpose

`basic_accessibility_audit` is a bounded, read-only inspection of selected PDF accessibility signals. It does not establish PDF/UA conformance, simulate assistive technology, remediate the PDF, or replace a qualified human review.

The public session command is `audit_session_accessibility`. The structure endpoint remains `get_session_structure_tree` because it returns a read-only structural inspection rather than a validation or editing capability.

## Out of scope

- Full PDF/UA-1 or PDF/UA-2 machine validation
- WCAG conformance
- Reading-order verification or editing
- Auto-tagging and alternative-text editing
- Unicode/ToUnicode conformance
- Color contrast, multimedia, mathematical and cognitive accessibility
- Persistence or attestation of human-review decisions

## Terminology and profiles

`pdf_ua1_reference` uses ISO 14289-1:2014/PDF 1.7 as the audit lens. `pdf_ua2_reference` uses ISO 14289-2:2024/PDF 2.0. A reference profile is not a conformance claim. An XMP declaration is evidence about the document's claim, not proof that the claim is true.

PDF/UA-2 namespace and MathML semantics are not fully implemented. A PDF/UA-2 or ambiguous profile therefore makes the affected coverage incomplete.

## Report schema

`AccessibilityAuditReport`, schema `accessibility-audit/2`, contains:

- `referenceProfile` and `summaryStatus`
- bounded `documentFacts`
- stable, individually classified `checks`
- explicit `coverage`, including implemented, unimplemented and incomplete IDs
- separate `humanReview` items
- bounded warnings and `truncated`

Summary states are `findings_detected`, `no_findings_in_implemented_checks`, `audit_incomplete`, and `audit_failed`. There is no percentage, grade, aggregate pass boolean, or conformance claim.

Check results are `pass`, `fail`, `warning`, `not_applicable`, `not_checked`, `indeterminate`, and `human_review_required`. A `pass` applies only to the narrow condition named by that check. Severity is triage metadata, not a score.

Frontend wrappers strictly validate schema versions, enums, unknown fields, collection sizes, string sizes, unique check IDs, coverage references, and the invariant `conformanceClaim === false` before committing results to UI state.

## Implemented document checks

- PDF header version, Catalog `/Version`, effective version and reference-profile selection
- Namespace-aware XMP `pdfuaid:part`, `pdfuaid:rev`, and `dc:title`
- StructTreeRoot presence/readability
- `/MarkInfo` dictionary presence, `/Marked` entry presence, `/Marked == true`, and their consistency with StructTreeRoot as separate facts
- Catalog `/Lang` decode, non-empty state, and bounded language-tag syntax
- XMP title, separately from supplemental Document Info `/Title`
- `/ViewerPreferences/DisplayDocTitle`, separately from title metadata
- Bounded structure integrity and Figure `/Alt` state
- Basic RoleMap resolution and cycles
- Basic ParentTree number-tree and declared `StructParents`/`StructParent` lookup integrity

Annotation inventory/semantics, forms, PDFium text availability, full content-stream association, Unicode mapping and encryption permission checks remain explicit `not_checked` checks in this version. Their absence from implementation is not treated as a pass.

## Structure parser

The cached worker-owned `lopdf::Document` is traversed with centralized depth, node, visited-object, child, string, evidence and deadline budgets. Active-path object IDs distinguish a cycle from a repeated occurrence. Every occurrence has a unique ID based on indirect object identity plus path, or a deterministic direct-object path.

`/K` supports integer MCID, indirect/direct StructElem, arrays, MCR, OBJR and mixed arrays. Unsupported objects, dangling references, MCR `/Pg` and OBJR `/Obj` failures are reported rather than silently omitted. A dangling `/K` reference is also retained as a deterministic `unknown`/`unresolved` child node. PDF strings use the PDF string decoder; invalid or oversized values do not become lossy display text.

The structure response, schema `accessibility-structure/1`, is a read-only summary. It is not a screen-reader preview, reading-order verification or PDF/UA result.

## RoleMap and namespaces

RoleMap dictionaries are bounded to 4,096 entries. Custom roles are followed through bounded chains; cycles, invalid targets, missing `/S`, and unresolved custom roles become findings. The implementation recognizes the common PDF 1.7 standard structure set. PDF 2.0 namespace semantics are not assumed from a matching name and remain incomplete.

## ParentTree and content association

The number tree is bounded and checks `Nums`, `Kids`, non-negative and duplicate integer keys, dangling values, cycles/duplicate nodes, child-count truncation, page `/StructParents`, and annotation `/StructParent` references. MCR and OBJR dictionaries are inventoried and their direct references are checked.

Full page/Form XObject content-stream MCID sequence validation is not implemented. The report says so through `structure.content-association = not_checked`; a syntactically consistent ParentTree is not presented as a verified reading order or semantic association.

## Figures, annotations, forms and text

Figure `/Alt` states distinguish missing, empty, whitespace-only, present and invalid. `/ActualText` is reported independently and does not satisfy the `/Alt` check. Alternative-text quality remains `human.figure-alt-quality`.

Annotation, form, text/Unicode and accessibility-extraction-permission categories have stable, visible `not_checked` controls and category coverage entries until they can be implemented against the worker-owned PDFium document and correlated safely with structural object identity. This prevents a successful structure parse from implying those categories passed.

## Human review

Every report lists logical reading order, semantic tag accuracy, alternative-text quality, artifact use, headings, tables, link purpose, language accuracy and changes, color/contrast, form interaction, OCR, math, multimedia and cognitive accessibility as human-review work. None is represented as passed.

## Resource limits

| Resource | Limit |
|---|---:|
| Accessibility source | 64 MiB |
| PDF objects for audit coverage | 100,000 |
| Structure depth | 128 |
| Structure nodes | 10,000 |
| Unique visited structure objects | 8,192 |
| Children per node | 4,096 |
| RoleMap entries | 4,096 |
| ParentTree entries | 50,000 |
| Number-tree depth | 64 |
| PDF string | 64 KiB |
| Display preview | 512 characters |
| Evidence per check | 32 |
| XMP compressed/decoded | 1 MiB each |
| Serialized audit report | 1 MiB |
| Serialized structure response | 4 MiB at the frontend boundary |
| Cooperative audit deadline | 12 seconds |
| Parent document-read timeout | 15 seconds |

Limit exhaustion sets `truncated`, records a bounded warning, and produces `audit_incomplete`; it never produces “no findings.” The initial `lopdf` load is protected by the worker process source, memory and hard-timeout boundaries but is not cooperatively interruptible internally.

## Worker and session architecture

The frontend sends only `sessionId` and `expectedContentGeneration`. The document-session broker resolves the already-open viewer worker record. That worker owns the PDFium document and a cached accessibility source. The active PDF is not reopened in the main Tauri process and credentials are not added to the audit payload.

The operation is read-only: it does not alter dirty state, revision, content generation, a working file, or an output file. Worker protocol v3 contains `AuditAccessibility`/`AccessibilityAudit` and `AccessibilityStructure`.

The UI commits a result only while its request sequence, session ID, content generation and mounted state still match.

## UI wording

The panel uses Summary, Checks, Structure, Human Review and Coverage. It does not show a score, pass badge, editing form, auto-tag action, drag/drop, or screen-reader simulation. Even a no-findings result is phrased only in terms of implemented checks.

## Test corpus

Small PDFs already used by worker integration tests cover normal, encrypted, credential, deleted-source-path, malformed-structure, restart, close-cleanup, lifecycle and worker ownership cases. Unit-generated deterministic object graphs cover document facts, declaration/profile matching, mixed `/K`, direct siblings, MCID/MCR/OBJR, dangling references, self- and multi-node cycles, non-cycle duplicate references, limits, RoleMap, ParentTree, XMP and Figure alternative states. The broker test helper supplies deterministic audit timeout, crash, protocol-mismatch and oversized-response modes. No fixture is downloaded at test time.

### Programmatic fixture matrix

| Fixture family | Purpose and expected checks | Expected summary/coverage | Human review |
|---|---|---|---|
| Untagged and MarkInfo combinations | Separate StructTreeRoot, MarkInfo dictionary, Marked entry and boolean value facts | Findings, or incomplete for unsupported PDF version; unchecked categories remain visible | Always outstanding |
| Complete implemented-check document | Valid PDF 1.7/UA-1 declaration, language, XMP title, DisplayDocTitle, structure and empty ParentTree | `no_findings_in_implemented_checks`; explicitly not full conformance | Always outstanding |
| Language/title variants | Missing, empty, invalid syntax, oversized string, title/preference cross-products | Relevant narrow check fails or is indeterminate; unrelated check remains independent | Language accuracy remains outstanding |
| Structure child corpus | Direct/indirect StructElem, MCID, MCR, OBJR, mixed arrays and dangling child | Finding for malformed/unresolved nodes; limits become `audit_incomplete` | Semantic structure and reading order remain outstanding |
| Cycle/duplicate corpus | Self-cycle, multi-node cycle and repeated non-active reference | Cycles are bounded findings; non-cycle duplicate occurrence is retained with a unique UI key | Reading order remains outstanding |
| RoleMap/ParentTree corpus | Valid mapping, missing `/S`, unmapped/cyclic/oversized RoleMap; valid, duplicate, dangling and oversized ParentTree | Findings or `audit_incomplete`; content-stream association remains `not_checked` | Semantic accuracy remains outstanding |
| Figure corpus | Missing, empty, whitespace, valid, oversized-invalid Alt; ActualText-only; many Figures | Narrow Alt-presence check only; no score | Alt quality remains outstanding |
| Worker malformed/lifecycle corpus | Cached source, encrypted/wrong credential, malformed structure, timeout, crash, protocol mismatch, oversized response, restart and close | Findings return a normal audit; operation/protocol failures return typed errors | Unchanged |

## Known limitations

- PDF/UA-2 namespaces and MathML are incomplete.
- ParentTree inspection does not parse complete marked-content sequences.
- Annotation/form object correlation with PDFium is not implemented.
- Text availability, scanned-page likelihood, Unicode mapping and encryption permission remain not checked.
- Independent machine validation and human-review persistence are separate future capabilities.
- `lopdf` decodes all representable PDF string byte sequences used by the deterministic language corpus; an artificial decode-failure fixture is therefore not asserted. Oversized strings remain deterministic `invalid` inputs.
