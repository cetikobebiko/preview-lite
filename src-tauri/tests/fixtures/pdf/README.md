# Atomic document I/O PDF fixtures

These deliberately small PDF 1.4 files are project-authored test fixtures and
are released under the repository license.

- `minimal.pdf`: one unannotated US Letter page.
- `three-distinct-pages.pdf`: three pages with distinct MediaBox sizes
  (`300x400`, `400x500`, and `500x600` PDF points) so reorder assertions can
  identify page order without rasterization.
- `annotations.pdf`: one page with one text annotation whose contents are
  `Fixture note`.
- `encrypted.pdf`: one US Letter page protected with the user password
  `fixture-password`; mutations must reject it until encryption preservation is
  verified. It was generated from `minimal.pdf` with pypdf 6.10.0 using
  RC4-128 solely as a compact compatibility fixture.
- `reorder-rich.pdf`: three pages with distinct text, dimensions, a 90-degree
  rotation on page two, and a text annotation on page three. It intentionally
  has no document metadata, outline, named destination, or AcroForm.
- `reorder-with-{metadata,outline,named-destination,form}.pdf`: one unsupported
  document-level feature per fixture. Extracting pages must reject these inputs
  before writing because PDFium page-copy does not preserve those structures. An
  in-place reorder keeps them: it rewrites only the page tree as an incremental
  update.
- `large-page-dimensions.pdf`: one 7200x7200 pt (100x100 in) page testing large
  page dimension handling without exceeding the 14,400 pt engine safety limit.
- `release-smoke.pdf`: two-page canonical smoke fixture with distinct text tokens,
  page markers, vector shapes, and a self-contained Type 3 Turkish Unicode font.

`generate_reorder_fixtures.py` is the deterministic, project-authored generator
for the reorder fixtures.
`generate_release_smoke.py` is the deterministic, project-authored generator
for `release-smoke.pdf` and `large-page-dimensions.pdf`. Tests consume only the committed PDF files.

The fixtures are static and require no network access or runtime PDF
generation. Their expected structure is validated by PDFium in the test suite
and was independently checked with strict pypdf parsing and Poppler rendering.

SHA-256:

- `minimal.pdf`: `0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d`
- `three-distinct-pages.pdf`: `55f8e4d2aedcfc3603c398c648d1a846e6eafe60f26e5c43c7288984f232914a`
- `annotations.pdf`: `43e4cbd366712cee2290f1de48c1cb1a476bed120e18bd074b62f418e9b5deda`
- `encrypted.pdf`: `4a07c1b65793257cf412dc5f478ea84525f07e94705bbbb084bb427cd241f28d`
- `reorder-rich.pdf`: `ec45cb69345f177f77c67073b7a5cb988d96276697ef34a35d0ae68eca5e492c`
- `reorder-with-form.pdf`: `4f4cb184e4cf4f342b83b3675e8360254ed1f366c0bb8834cc5ff085a8ec58cf`
- `reorder-with-metadata.pdf`: `8825c0d1abecd665c1e89f882ad811237cdbadbf7c07e1a9198600754a28af65`
- `reorder-with-named-destination.pdf`: `241264be3211c3d50376d77f7136fd3cfa1d9f53791a2611a5475c52e667f325`
- `reorder-with-outline.pdf`: `e2dff7f1786fac45ad65fb70bf38e7842fefda6b5550b128e33eef9c96a5fe96`
- `large-page-dimensions.pdf`: `93cb5f67e9587658f60a1db12a5f29aecf29d6b8ec47403400d9a8f48406fa55`
- `release-smoke.pdf`: `727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611`
