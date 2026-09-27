# Print Resource Inspection and RGB-Derived Preview

## Capability boundary

`print_resource_inspection` is a read-only, experimental observation of PDF dictionaries and resource definitions. It does not prove visible use, printing outcome, print readiness, or PDF/X conformance. Every report sets `conformanceClaim: false` and exposes inspected, not-inspected, incomplete, and limited areas.

`rgb_derived_print_preview` is an experimental display aid. PDFium renders display RGB pixels; the worker then applies deterministic naive RGB-to-C/M/Y/K math. Metadata always states `approximate: true`, `colorManaged: false`, `outputIntentApplied: false`, `spotColorsPreserved: false`, and `overprintSimulated: false`. It is not a plate, real separation, RIP simulation, Total Area Coverage measurement, or ink-consumption estimate.

## Session and worker lifecycle

Both operations require `jobId`, `sessionId`, `expectedContentGeneration`, `physicalPageIndex`, and `requestId`. The main process creates an immutable private snapshot and launches the existing isolated job worker. The worker constructs result identity from the authoritative session context. Frontend runtime validation rejects unknown schemas, malformed arrays/numbers/base64 sizes, unsupported methodology metadata, and stale identity.

Replacement, page/session/generation change, panel close, document transition, and unmount cancel the active job and invalidate its request ID. Preview workers receive a private cancellation marker and check it before rendering, in bounded scanline groups, before and between PNG encodes, and before serialization. A cancelled job does not emit a ready event.

## Inspection coverage

The bounded, deterministic, cycle-safe traversal covers direct and inherited page resources, nested Form XObjects, Image XObjects, Patterns, Shadings, ColorSpace dictionaries, alternate color spaces, OutputIntents, Font resources, ExtGState, and effective page boxes. Definitions, unique indirect objects, references, and affected pages are distinct concepts. Content-stream use, inline images, annotations, optional content, and external references are currently `not_inspected`; observed definitions use `usage_not_checked`.

ICCBased streams receive bounded `basic_header_only` inspection: presence, encoded/decoded size, digest, component count, alternate, declared size, `acsp`, version, device class, input space, and PCS where available. Tag correctness and transform usability are not checked. Encrypted snapshots are explicitly unsupported for structural inspection when lopdf cannot safely participate in the session credential model; PDFium-based previews can still use worker-private credentials.

## Limits

Inspection limits cover pages, objects, depth, resource references, color spaces, ICC count and bytes, OutputIntents, spots, fonts, findings, and report/worker response size. Preview validates finite scale and threshold, page bounds, dimensions, overflow-safe pixel count, each PNG size, combined encoded size, and runtime timeout. Limit exhaustion is an incomplete/typed failure, never a clean result.
