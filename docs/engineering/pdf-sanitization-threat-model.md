# PDF Sanitization Threat Model and Capability Characterization (8C.1)

## 1. Amaç, kapsam ve karar

Bu belge üretim sanitizer'ı değildir. Repository'nin gerçek `lopdf 0.34.0`,
`pdfium-render 0.8.37` ve vendored PDFium `152.0.7961.0` yeteneklerini kaynak ve
deterministik fixture testleriyle sınırlar. Amaç; neyin incelenebildiğini, hangi dar
kontrollerin gelecekte transform adayı olduğunu ve hangi içeriğin fail-closed blocker
kalacağını kanıtlamaktır.

> **8C-1 COMPLETE — THREAT MODEL AND SUPPORT MATRIX ESTABLISHED — 8C-2 READY**

Bu karar activation değildir. `sanitization.status == "disabled"`,
`sanitization.enabled == false` ve `sanitize_pdf_document == FEATURE_UNAVAILABLE`
değişmemiştir. Inspection IPC, production scanner/transform/verifier, output,
publication, Save dialog veya production DTO bu aşamada yoktur.

8C, gelecekte kurulacak şu document-wide pipeline'dır:

```text
inspect → selectively remove → full rewrite → independently verify → atomically publish
```

8C redaction, OCR veya malware scanner değildir ve bütün PDF tehditlerini bulma
garantisi vermez. **8A** sensitive-data detection/geometry foundation; **8B** yalnız
kullanıcının seçtiği dikdörtgenler için destructive secure-redaction export;
**8C** tanımlı active-content/privacy control pipeline'ıdır. 8B DTO/policy'si 8C
policy'si değildir. 8B'nin session ownership, worker isolation, credential transport,
digest-bound publication ve atomic `document_io` primitive'leri ileride generic
altyapı olarak değerlendirilebilir; bu aşamada değiştirilmemiştir.

## 2. Güvence seviyeleri

* **Inspection:** Belgeyi değiştirmeden yalnız desteklenen kategorileri inceler.
  Başka aktif içerik olmadığını, bütün payload'ların bulunduğunu veya unsupported
  kategorilerin güvenli olduğunu söylemez.
* **Custom cleanup:** Kullanıcının seçtiği candidate kontrollerdir. Sonuca “PDF
  temizlendi”, “güvenli PDF” veya “zararsız PDF” denemez.
* **Verified limited sanitization:** Yalnız mandatory minimum profile uygulanmış,
  blocking unsupported içerik yok, full rewrite/history removal kanıtlanmış,
  bağımsız verifier geçmiş ve artifact atomik yayımlanmışsa kullanılabilir.
  Mandatory `not_run` publication'ı engeller. 8C.1 bu güvenceyi üretmez.

## 3. Mevcut call graph ve legacy migration debt

```text
SanitizationDialog.svelte
 → guardFeatureAction("sanitization")
 → AppModalHost / App.svelte onSanitize
 → featureRuntime.sanitize
 → documentTransformController.sanitizeDocument
 → context.path.replace(".pdf", "_sanitized.pdf")
 → sanitizePdfDocumentAction (ikinci feature guard)
 → backend sanitizePdfDocument wrapper / runSessionJob
 → sanitize_pdf_document
 → sanitize_pdf_document_unavailable()
 → FEATURE_UNAVAILABLE
```

Registry gate nedeniyle production frontend çağrısı backend'e ulaşmaz. Backend
command doğrudan parametresiz helper'a döner. Architecture testleri command/helper
gövdelerinde snapshot, parser, PDFium, `lopdf`, worker veya filesystem çağrısı
olmadığını kilitler. Snapshot, parse, worker, output, external target teması ve
session mutation yoktur.

Legacy Rust/wire model şudur:

```text
SanitizationOptions {
  remove_metadata,
  remove_annotations,
  remove_javascript,
  remove_embedded_files,
  remove_hidden_layers
}
```

`SanitizationDialog`, `sanitizePdfDocumentAction`, backend wrapper,
`documentTransformController`, featureRuntime ve App/modal wiring bu dormant geniş
boolean contract'ı taşır. Gelecek 8C authority'si değildir. `_sanitized.pdf` implicit
target atomic publication/kullanıcı seçimi sağlamaz ve reuse edilmemelidir.
`remove_hidden_layers` yanlış abstraction'dır: `/OCProperties` silmek hidden content
silmek değildir. 8C.2 inspection bu legacy payload'ı kabul etmeyecek; 8C.6 frontend
migration bunu versioned workflow ile değiştirecektir.

## 4. Support status modeli

Yalnız `supported_candidate`, `supported_with_visual_change_candidate`,
`supported_with_functional_change_candidate`, `experimental_candidate`,
`unsupported`, `not_applicable`, `unverified` kullanılır. Candidate production
transform ve independent verifier acceptance tamamlanmadığını ifade eder.

## 5. Threat model ve support matrix

| category | PDF object paths / representations | detection capability | mutation/removal capability | pruning requirement | verification capability | visual impact | functional impact | current support decision | blocking behavior | evidence/tests |
|---|---|---|---|---|---|---|---|---|---|---|
| `document_javascript` | Catalog `/OpenAction`, `/AA`, `/Names /JavaScript`; `/S /JavaScript`; `/JS` literal, hex, stream, indirect; `/Next` | Direct/indirect ve payload biçimleri ayrışır; destination-array negatif kontrolü var | Targeted action/name-tree edge; bütün `/Names` silinmez | Evet | Path/type, string, decoded stream, digest, raw ek scan | Yok | Automation durur | `supported_candidate` | Unresolved/remaining finding block | JS, destination ve combined Dests/JavaScript/EmbeddedFiles sibling tests |
| `additional_actions` | Catalog/Page `/AA`; Page `/O` `/C`; annotation/widget/field `/A` `/AA`; field `/K /V /C /F`, `/CO` | Key ve action type direct/indirect görülür | Yalnız action entries; form field korunabilir | Detached action için | Independent action graph rescan | Yok | Page/form events durur | `supported_with_functional_change_candidate` | Limit/unresolved block | JS fixture, bounded walker |
| `annotation_actions` | Annotation/Widget/Link `/A`, `/AA` | Annotation varlığı action'dan ayrı | Link korunup action kaldırılabilir; payload cleanup değildir | Evet | Annotation retained + forbidden action absent | Genelde yok | Link/automation durur | `supported_with_functional_change_candidate` | Failure block | JS/network fixtures |
| `form_actions` | `/AcroForm`, field/widget `/A` `/AA`, `/CO` | Keystroke/validate/calculate/format fixture'da | Form action removal, all-fields removal değildir | Evet | Field inventory + action rescan | Yok | Validation/calculation durur | `supported_with_functional_change_candidate` | Unresolved field tree block | JS fixture |
| `all_form_fields` | `/Fields`, Widget/AP/inheritance | Inventory mümkün | Appearance/inheritance güvenliği yok | Kapsamlı | Yok | Yüksek | Form kaybolur | `unsupported` | Minimum blocker değil; control execute edemez | Decision |
| `launch_actions` | `/S /Launch`, `/F`, `/Win`, nested `/Next` | Nested/cycle içinde bulunur | Action edge targeted kaldırılabilir | Action/FileSpec | Forbidden type + reachability | Yok | Local launch durur | `supported_with_functional_change_candidate` | Failure block | network fixture |
| `uri_actions` | `/S /URI`, Link `/A` | JS'den ayrı | URI action kalkar, Link kalabilir | Evet | `/S /URI` absent | Yok | Web link durur | `supported_with_functional_change_candidate` | Failure block | network fixture |
| `remote_navigation_actions` | `/GoToR`, `/GoToE`, file spec, destination; local `/GoTo` ayrı | Remote/local ayrımı korunur | Targeted action edge | Evet | Type + external FileSpec scan | Yok | Remote nav durur | `supported_with_functional_change_candidate` | Failure block | network fixture |
| `form_submission_actions` | `/SubmitForm`, `/ImportData`, URL/FileSpec | Ayrı türler deterministic | Targeted action edge | Evet | Forbidden type rescan | Yok | Submit/import durur | `supported_with_functional_change_candidate` | Failure block | network fixture |
| `rendition_actions` | `/Rendition`, Screen/media clip | Üst type görülür | Exhaustive asset graph yok | Karmaşık | Yok | Olası | Media durur | `unsupported` | Minimum blocker | unsupported fixture |
| `embedded_files` | `/Names /EmbeddedFiles`, FileSpec, `/EF`, `/RF`, `/EmbeddedFile` | Branch/FileSpec/streams ayrı; combined sibling fixture aynı `/Names` içindedir | Bütün owner edges kaldırılınca candidate; Dests/JavaScript sibling'leri korunabilir | Zorunlu | Path/graph/decoded digest/raw ek scan | Attachment UI | Ekler kaybolur | `supported_with_functional_change_candidate` | Unresolved edge block | AF partial/complete prune ve combined-name-tree tests |
| `associated_files` | Catalog/Page/Annotation/Structure `/AF` | Dört owner ayrı; Structure `/AF` direct ve indirect FileSpec çözer | Her owner edge ayrı; shared FileSpec object-ID dedup karakterize | Zorunlu | No reachable `/AF`/FileSpec/payload | Olası | Association kaybolur | `supported_with_functional_change_candidate` | Limit/unresolved block | `structure_element_af_resolves_direct_and_indirect_filespecs_without_duplicate_inflation`; AF complete-prune test |
| `file_attachment_annotations` | `/Subtype /FileAttachment`, `/FS`, `/AF` | Subtype/FileSpec görülür | Subtype-specific removal; all-annotation değil | Zorunlu | Subtype/path/payload scan | İkon kaybolur | Attachment erişimi kaybolur | `supported_with_visual_change_candidate` | Failure block | AF fixture |
| `portfolio_collection` | Catalog `/Collection`, embedded name tree | Ayrı görülür | Embedded control ile birlikte | Payload için | Collection + embedded scan | Olası | Portfolio UI kaybolur | `supported_with_functional_change_candidate` | Embedded control ile block | AF fixture |
| `document_info` | Trailer `/Info` | Ayrı path/string | Trailer edge removal | Evet | Trailer + digest scan | Yok | Properties kaybolur | `supported_candidate` | Failure block | metadata fixture |
| `xmp_metadata` | Catalog `/Metadata` stream | Path/type/decoded stream | Targeted owner edge | Zorunlu | Path + digest | Yok | XMP kaybolur | `supported_candidate` | Failure block | metadata fixture |
| `page_metadata` | Page `/Metadata` | Page tree scan | Targeted page entry | Zorunlu | Every-page rescan | Yok | Page properties kaybolur | `supported_candidate` | Failure block | metadata fixture |
| `object_metadata` | Image/Form XObject ve diğer `/Metadata` refs | Image fixture + generic bounded walker | Targeted owner entry | Zorunlu | All-object path + digest | Yok | Object properties kaybolur | `supported_candidate` | Limit/unresolved block | metadata/object-stream fixtures |
| `private_application_data` | `/PieceInfo`, `/LastModified`, app-private dictionaries | Bilinen keys | Bilinen allowlist paths | Olası | Universe exhaustive değil | Yok | Round-trip data kaybı | `experimental_candidate` | Minimum dışında; seçilemez | metadata fixture |
| `document_identifier` | Trailer `/ID` | Ayrı fact | Preserve/replace/remove teknik olarak mümkün | Hayır | Equality/absence | Yok | Identity/cache semantics | `supported_candidate` | Policy mismatch block | ID writer tests |
| `embedded_file_metadata` | FileSpec `/F`, `/UF`, `/Desc`, `/Params`, dates | Ayrı görülür | Embedded control ile kaldırılır | Evet | FileSpec/payload absent | Yok | Name/desc kaybolur | `supported_with_functional_change_candidate` | Embedded control ile block | AF fixture |
| `xfa` | `/AcroForm /XFA` single indirect stream ve name/stream array refs | Single-stream ve array fixture'larında presence, resolution ve reachability görülür | Controlled fixture edge+prune streams çıkarır; dynamic/script graph exhaustive değil | Zorunlu | Genel verifier yok | Yüksek | Dynamic form kırılır | `unsupported` | Minimum blocker | `xfa_single_reference_resolves_stream_and_prunes_after_edge_removal`; `xfa_array_references_resolve_streams_and_prune_after_edge_removal` |
| `rich_media` | RichMedia, Screen, Movie, Sound, 3D, Rendition, assets/external refs | Üst subtype ve controlled asset | Tüm relations enumerate edilmedi | Zorunlu | Yok | Yüksek | Multimedia durur | `unsupported` | Minimum blocker | unsupported fixture |
| `optional_content` | `/OCProperties`, OCG, OCMD, marked-content refs, visibility, annotation `/OC` | OCG ve OCMD type'ları, resource refs ve layer-associated annotation ayrı görülür | `/OCProperties` removal marked content, OCMD ve annotation `/OC` izlerini kaldırmaz | Dictionary prune yetersiz | Görsel/semantic verifier yok | Yüksek | Layer semantics bozulur | `unsupported` | Minimum blocker | `optional_content_ocg_ocmd_and_annotation_oc_are_independently_detectable`; removal-insufficiency test |
| `all_annotations` | `/Annots`, subtype/AP/payload | Inventory mümkün | Tüm subtype safety yok | Kapsamlı | Yok | Yüksek | Yorum/form/media kaybı | `unsupported` | Minimum blocker değil; active subtype kendi blocker'ı | subtype corpus |
| `incremental_revisions` | `/Prev`, multiple classic xref, real xref-stream chain, hybrid | Raw+parser facts; latest graph ve physical previous payload ayrışır | Explicit cleanup+prune+full save | Zorunlu | Single EOF/no history/old digest/output-local decoded xref/reopen | Yok | History gider | `supported_candidate` | Proof yoksa block | classic ve xref-stream-chain full-rewrite tests |
| `encryption` | `/Encrypt`, `/ID`, handler/permissions | lopdf/PDFium credential facts | Untouched save korur; decrypt-save unencrypted; mutation preservation güvenli değil | Özel | Security semantics verifier yok | Yok | Protection değişebilir | `unsupported` | `SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED` | encryption tests |
| `digital_signatures` | Sig field/dict, `/ByteRange`, DocMDP, FieldMDP | lopdf paths + PDFium count | Full rewrite invalidate eder | N/A | Crypto verifier yok | Olası | Certification bozulur | `unverified` | Explicit acknowledgement yoksa block | signature fixture |
| `standard_conformance` | PDF/A, PDF/X, PDF/UA, certified, linearized | Compliance validator yok | Preservation iddia edilemez | Değişken | Yok | Değişken | Conformance kaybı | `not_applicable` | Assurance conformance iddiası taşımaz | Policy |

## 6. Action, metadata ve embedded-file policies

`/OpenAction` yalnız dictionary/reference olup `/S /JavaScript` taşıdığında JS action
sayılır. `[page /Fit]` destination action değildir. `/Names /JavaScript` branch'i
`/Dests` ve `/EmbeddedFiles` branch'lerinden bağımsız ele alınır; bütün `/Names`
silinmez. Action walker yalnız `/Next` single dictionary/reference veya array'i,
visited set ve depth/object limitleriyle izler. `URI`, `GoToR`, `GoToE`,
`SubmitForm`, `ImportData`, `Launch`, `Rendition` JS kontrolüne katılmaz. Local
`GoTo` remote değildir. Corpus'un PDF 1.7 page additional-action dictionary'si için
geçerli page-level keys `/O` (open) ve `/C` (close) olarak ayrı allowlist edilir;
annotation/field AA keys bu page allowlist'ine karıştırılmaz. Daha yeni PDF sürümü
veya extension level kabul edilirse 8C.2 allowlist'i version-aware fail-closed
güncellenmelidir.

Metadata tek boolean değildir: Info, document/page/object XMP, ID, private data ve
embedded-file metadata ayrı finding/control'dür. Writer testi mevcut `/ID`'yi korur,
explicit kaldırılmış ID'yi eklemez ve ID'siz document'a otomatik ID üretmez.
`preserve`, explicit `regenerate` ve PDF policy izin veriyorsa `remove_if_permitted`
teknik olarak tasarlanabilir; production policy 8C.3 işidir.

Yalnız `/Names /EmbeddedFiles` kaldırmak yeterli değildir. Catalog/Page `/AF` ve
FileAttachment `/FS` payload'ı reachable tutar. Partial removal marker'ı bırakır;
bütün owner edges kaldırılıp prune edilince FileSpec/EmbeddedFile çıkar. Unresolved
AF/FileSpec/EF/RF silent ignore edilemez.

## 7. XFA, RichMedia ve optional-content policies

Single indirect-stream ve array XFA fixture'larında `/XFA` edge removal+prune
controlled streams'i çıkarır; bu dynamic XFA/script graph completeness kanıtı
değildir. XFA unsupported/blocker'dır.
RichMedia/Rendition/Screen/Movie/Sound/3D üst düzey tespit edilebilir, tüm asset ve
external relations kanıtlanmadığı için unsupported/blocker'dır.

```text
remove /OCProperties != remove hidden content
```

Testte `/OCProperties` kaldırılıp prune edilse bile optional marked-content stream
marker'ları, OCMD ve annotation `/OC` graph içinde kalır. Catalog taraması bu page
resource/annotation ilişkileri için yeterli değildir. Content rewrite/güvenli
flattening yoktur. `optional_content = unsupported`; legacy
`remove_hidden_layers` production policy'den kaldırılmalıdır.

## 8. Pinned library capability matrix

| Capability | `lopdf 0.34.0` sonucu | PDFium / `pdfium-render 0.8.37` rolü | 8C sonucu |
|---|---|---|---|
| Classic xref/direct objects | Parse PASS | Reopen/render PASS | Inspection primitive |
| Object stream | Compressed object resolve PASS | Reopen/render PASS | Save compressed objects'i expand edebilir |
| Xref stream | Parse PASS | Reopen/render PASS | Inspection primitive |
| Incremental xref-stream chain | Revision 2 stream `/Prev` → Revision 1 stream; latest object resolution ve physical old marker PASS | Full rewrite output reopen/render PASS | `supported_candidate`; explicit trailer normalization ve decoded output-xref proof zorunlu |
| Hybrid xref | Table + `/XRefStm` + `/Prev` fixture PASS; merge sırasında parser trailer `XRefStm` key'ini tüketir | Reopen/render PASS | Raw ve parsed facts birlikte gerekir |
| Incremental revisions | `/Prev` merge ve old object access PASS | Reopen PASS | Explicit history cleanup gerekir |
| Strings/streams | Literal/hex access ve decoded stream PASS | Arbitrary JS graph API'si yok | lopdf structural authority |
| Name-tree primitives | Dictionary/array/reference PASS | Attachment convenience API exhaustive değil | Branch-specific scanner mümkün |
| Stream dictionary mutation | PASS | Arbitrary dictionary mutation yok | Candidate mutation primitive |
| Object deletion/replacement | Map replacement ve `delete_object` mevcut | Page/annotation convenience APIs sanitizer authority değil | Semantic policy zorunlu |
| Reachability pruning | Fixture, shared object ve cycles PASS | Arbitrary prune API yok | Mandatory transform step |
| Full serialization | Baştan save; stale `/Prev` otomatik temizlenmez | `save_to_writer()` flags=0, security/full-rewrite authority değil | Explicit trailer cleanup + verifier |
| Trailer mutation | PASS | Arbitrary trailer API yok | Info/ID/history policies mümkün |
| Document ID | Preserve; explicit remove; auto-add yok | Policy API yok | Candidate control |
| Dangling refs | Serialize/load mümkün, resolution sonra hata | Recovery değişebilir | Every-reference resolution mandatory |
| Cyclic graph | lopdf reachability ve bounded test walker cycle-safe | Arbitrary graph traversal yok | Explicit limits zorunlu |
| Encryption | Graph parse, `is_encrypted`, decrypt PASS; untouched save korur; decrypt+save unencrypted | Credentialed reopen ve Revision2 fact PASS | Production encrypted input unsupported |
| Signature facts | Structural paths PASS | Signature count/DocMDP bindings mevcut | Inspection cross-check; crypto validation değil |
| Final evidence | Parsed graph/strings/decoded streams/raw scan | Page count/dimensions/render smoke | Independent two-parser verification |

Kaynak incelemesi pinned `lopdf` reader'ın `/Prev`/hybrid merge kodunu, writer'ın
loaded trailer'ı yazdığını ve ObjStm/XRef/Linearized objects'i doğrudan kopyalamadığını,
processor'ın reachability/delete primitive'lerini doğrular. Genel crate bilgisine
dayanılmaz.

PDFium final reopen, page count/dimensions, render smoke, signature count,
credentialed encrypted reopen/security-handler revision ve mevcut page/annotation
facts için ikinci parser/render authority'sidir. Raw C symbol varlığı ownership,
arbitrary dictionary mutation, pruning veya payload safety kanıtı değildir. PDFium
arbitrary object-graph sanitizer değildir; render/geometry authority'si olarak kalır.

## 9. Full-rewrite characterization ve incremental policy

`Document::save()` input sonuna append etmez; nesneleri baştan serialize eder. Fakat
loaded incremental trailer'daki `/Prev` otomatik temizlenmez. Stale input offset'i
output'a yazılır ve controlled fixture reopen edilemez. Required future sequence:

1. supported owner edges'i kaldır;
2. payload/object fingerprints al;
3. `prune_objects()` uygula;
4. `/Prev` ve history-only `/XRefStm` state'ini explicit temizle;
5. private staging path'e full save;
6. bağımsız raw/graph/decoded scans ve parser/PDFium reopen yap.

Executable proof hem classic iki-revision input'ta old JavaScript/current metadata
edge'lerini hem gerçek xref-stream zincirinde metadata edge'ini kaldırır. Output
input prefix'i değildir, tek `%%EOF` taşır, `/Prev` taşımaz, unique markers
raw/graph'ta yoktur ve parser/PDFium açar. Classic output helper her subsection ve
free/in-use entry'yi ayrıştırır; xref-stream output helper `/W` ve `/Index` alanlarını
decode eder. Her in-use entry yalnız yeni output içindeki doğru
`<object-number> <generation> obj` header'ına işaret eder; eski absolute xref
authority'si reuse edilmez. Belirli xref biçimi değil, output-local valid offsets ve
history absence security invariant'idir.

## 10. Object-pruning proof ve malformed graph davranışı

Test graph'i reachable A, unreachable sensitive stream B, unreachable metadata B2
ve A'nın paylaştığı reachable C içerir. Prune B/B2 object ID'lerini döndürür; raw
markers yoktur; A/C ve shared marker kalır. Object count tek başına kanıt değildir.
Embedded testte partial edge removal payload'ı bırakır, bütün AF/annotation edges
kaldırıldığında FileSpec/stream prune olur.

`delete_object()` references'i mekanik kaldırabilir ama semantic owner policy değildir.
`prune_objects()` trailer reachability kullanır ve cycles'i visited IDs ile sınırlar.
Reachable `/OpenAction 99 0 R` ve `/Names /EmbeddedFiles 98 0 R` taşıyan fixture
parser tarafından açılır ve sayfa ağacı okunur; bounded walk iki unresolved critical
reference'i de bulur. Parser-open başarısı graph completeness/safety değildir.
Verifier her critical reference'i resolve etmeli, silent ignore etmemeli ve
unresolved finding minimum profile'ı block etmelidir.

## 11. Removed-payload evidence ve independent verification

Future verifier birlikte şunları kullanır: forbidden object type/path scan, string
object scan, bounded decoded-stream scan, transform öncesi payload SHA-256 digest
scan, raw serialized byte search ve every-reference/page-tree invariants.

```text
raw byte search alone != verification
```

Encoding/compression marker'ı gizleyebilir; benign bytes false positive olabilir.
Report payload değil digest ve bounded path evidence taşır.

Minimum architecture:

```text
Worker A — Transform
  parse immutable snapshot → inventory → targeted mutate → prune
  → explicit history cleanup → full rewrite → transform evidence

Worker B — Verify
  independent reopen → object graph/forbidden scan → payload digest scan
  → history proof → lopdf reopen → PDFium reopen/render → invariants

Coordinator
  validate mandatory checks/digest → atomic document_io publication
```

Worker A'nın “successful” boolean'ı verification authority değildir. Main process
`sanitize_pdf_document` içine `Document::load`, PDFium open veya object scan
eklenemez; 8C.2 session snapshot'ını ephemeral worker açmalıdır. Traversal visited
indirect set, depth/object budget, cycle detection, direct+indirect objects, arrays,
dictionaries ve stream dictionaries taşır.

## 12. Candidate minimum profile ve blocking policy

```text
document_javascript
additional_actions
launch_actions
remote_navigation_actions
uri_actions
form_submission_actions
embedded_files
file_attachment_annotations
document_info
xmp_metadata
object_metadata
document_identifier
```

`form_actions`, additional-actions ailesinin mandatory alt kapsamıdır. Associated
files/portfolio paths embedded-files control'ünün mandatory reachability kapsamına;
page metadata metadata family kapsamına girer.

Profil exhaustive değildir. `xfa`, `rich_media`/rendition/multimedia ve
`optional_content` detected ise execution/publication block edilir. Encrypted input
block edilir. Signed input explicit invalidation acknowledgement gelene kadar block
edilir. `all_annotations`, `all_form_fields` ve private application data minimum
profile dışında kalır; istenirse unsupported/experimental error üretir. Silent
ignore yasaktır.

## 13. Proposed contracts (design only)

```text
SanitizationPolicy {
  schemaVersion, profile, controls, acknowledgements, expectedInspectionVersion
}
```

Control IDs versioned allowlist olmalıdır. Unknown/duplicate fail-closed; unsupported
execute edemez; legacy broad booleans alias olamaz.

```text
SanitizationInspectionReport {
  schemaVersion, inspectionVersion, documentFacts,
  controls, blockingFindings, warnings, limits
}
```

Document facts en az `encrypted`, `signed`, `incrementalRevisionsDetected`,
`xfaDetected`, `optionalContentDetected`, `richMediaDetected`, `pageCount` taşır.
Control item: `controlId`, `support`, `present`, `findingCount`, visual/functional
impact, `blockingReason`.

Verification mandatory checks:

```text
fullRewrite, incrementalHistoryRemoved, objectGraphScan, removedPayloadScan,
parserReopen, pdfiumReopen, pageTreeValid, pageCountValid, renderSmoke,
unresolvedFindings
```

Her check `passed | failed | not_run`; mandatory `not_run` publication blocker'dır.

## 14. Encryption, signature ve conformance policies

Credential presence encryption authority değildir; actual `/Encrypt` ve PDFium
handler fact kullanılır. Controlled fixture'da doğru parola inspect/decrypt eder.
Untouched object-ID-preserving save encrypted açılır; decrypt sonrası `/Encrypt`
kalkar ve output plaintext olur. Mutated objects, permission/security-handler,
renumbering ve preservation bütünlüğü kanıtlanmadığı için future error:

```text
SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED
```

Full rewrite signature byte rangesini/certification'ı invalidate eder. Signature
field/dict, `/ByteRange`, DocMDP, FieldMDP inspect edilir; cryptographic validation
iddiası yoktur. Future signed execution için `explicit signature invalidation
acknowledgement` zorunludur.

PDF/A, PDF/X, PDF/UA, certified PDF ve linearization otomatik korunmuş sayılmaz.
8C verifier compliance validator değildir; verified-limited bu standartlara ilişkin
iddia taşımaz.

## 15. Resource-limit proposal for 8C.2

Production constants değildir; adversarial corpus ile kalibre edilecek başlangıçtır:

| Limit | Proposal | Repository budget ilişkisi |
|---|---:|---|
| max object count | 100,000 | Worker memory'den önce fail-closed |
| max traversal depth | 64 | Cycle set'e ek bound |
| max name-tree depth | 32 | Degenerate trees ayrı budget |
| max action-chain depth | 64 | Nested/cycle fixture'ın üstünde, unbounded değil |
| max decoded stream/object | 16 MiB | Tek stream amplification sınırı |
| max total decoded bytes | 128 MiB | 256 MiB job response/document tavanının yarısı |
| max embedded-file count | 1,024 | Inventory/report amplification |
| max findings | 10,000 | Existing sensitive-scan item tavanıyla aynı |
| max report bytes | 1 MiB | Existing job request tavanıyla aynı |

Mevcut job request 1 MiB, response 256 MiB; long-lived worker request 64 KiB;
credential document 256 MiB; accessibility document 64 MiB'dir. Limit aşımı partial
success değil explicit blocking error/finding'dir.

## 16. Report privacy

Report/log JavaScript source, full metadata values, embedded bytes,
password/credential veya arbitrary stream contents taşımaz. Allowed evidence:
category, bounded object path/id, count, digest, length/type gibi bounded safe facts
ve hit edilen limit. Fixture markers sentetiktir; user payload loglama modeli değildir.

## 17. Fixture corpus, manifest ve determinism

`src-tauri/tests/fixtures/sanitization/` standard-library-only generator ve strict
manifest içerir. On altı birleşik fixture, başlangıç corpus'una ek closure vakalarını kapsar:
clean, Info, catalog/page/object XMP, JS name tree, JS/destination OpenAction,
Catalog/Page AA, annotation/field/widget actions, nested/cyclic Next,
Launch/URI/GoToR/GoToE/SubmitForm/ImportData, EmbeddedFiles/EF/AF/FileAttachment,
Structure `/AF`, single/array XFA, OCG/OCMD/annotation `/OC`, aynı dictionary'de
Dests+JavaScript+EmbeddedFiles, classic incremental, real incremental xref-stream
chain, object/xref stream, hybrid, dangling critical refs, encryption ve
signature/ByteRange/DocMDP/FieldMDP.

Required manifest fields: `name`, `purpose`, `threatCategories`,
`expectedInspection`, `candidateControls`, `blockingUnsupported`,
`expectedVisualImpact`, `expectedFunctionalImpact`, `futureTransformExpectation`,
`futureVerificationExpectation`, `notes`. `sha256`/`byteSize` generation binding
sağlar. Serde `deny_unknown_fields` unknown/missing alanları reddeder. Generator iki
temp directory'de çalıştırılır; manifest dahil bytes eşitliği test edilir. IDs,
dates/credential fixed; timestamp/random UUID ve internet/user data yoktur.

## 18. Characterization tests

`src-tauri/tests/sanitization_capabilities.rs`: manifest/determinism; parser matrix;
bounded graph/action cycles; name-tree independence/Dests retention; metadata paths;
embedded/AF partial-complete pruning ve Structure `/AF` dedup; combined name-tree
sibling preservation; XFA single/array resolution; OCG/OCMD/annotation `/OC`;
optional-content invariant; shared-object pruning; stale-Prev, classic/xref-stream
full rewrite ve output-local xref decode; ID behavior; dangling critical refs;
encryption; signature; PDFium reopen/dimensions/render/signature/security facts.

`src/sanitizationArchitecture.test.js`: registry disabled state, backend fail-closed
boundary ve dormant broad-booleans/implicit target debt. Existing backend architecture
ve feature guard suites snapshot/parser/filesystem/worker yokluğunu da korur.

## 19. Known limitations ve 8C.2 blockers

1. Versioned inspection DTO/worker protocol ve production bounded scanner yok.
2. Page-AA version-aware allowlist tamamlanmalı.
3. Deep name-tree Kids/Limits ve daha geniş adversarial/malformed corpus tamamlanmalı.
4. Private/object metadata allowlist ve unknown-path policy tamamlanmalı.
5. Limits adversarial/large corpus ile kalibre edilmeli.
6. Candidate mutation algorithms/shared-owner semantics test edilmeli.
7. Transform ve bağımsız verifier workers yok.
8. Production history cleanup/xref variants acceptance yok.
9. Encryption error ve signature acknowledgement contracts yok.
10. XFA/RichMedia/optional content blocker kalır.
11. Atomic staging/publication/digest binding 8C için kurulmadı.
12. Platform runtime certification yapılmadı.

Detection foundation 8C.2 için yeterlidir: candidate ve unsupported blocker families
deterministic bulunabilir, limits/unresolved facts fail-closed raporlanabilir. Bu
transform/publish readiness değildir.

## 20. Değişmez sınırlar

Production `sanitize_pdf_document` içine parsing/I/O eklenmemiş, frontend contract
migrate edilmemiş, output üretilmemiş, feature açılmamış ve 8B secure-redaction
implementation/whole-object NO-GO kararı değiştirilmemiştir. Başarı yalnız capability
ve blocker sınırlarının kanıtlanmasıdır.

## 21. 8C.2 Inspection Implementation Status

8C.2 read-only inspection pipeline uygulanmıştır; sanitization transform, bağımsız
verification ve publication uygulanmamıştır. Feature registry hâlâ
`sanitization.status == "disabled"` ve `enabled == false` taşır. Legacy
`sanitize_pdf_document` parse, snapshot, worker veya I/O yapmadan
`FEATURE_UNAVAILABLE` döndürmeye devam eder.

Public `inspect_document_sanitization` command yalnız `job_id`, `session_id`,
`expected_content_generation` ve `policy_schema_version` alır. Raw input/working/
snapshot path, credential, output veya target path public IPC contract'ında yoktur.
Command mevcut `run_snapshot_job` mekanizmasıyla immutable physical snapshot üretir;
session ID/content generation/state revision/dirty/working path/save target değişmez.
Ephemeral job worker'ın private `InspectDocumentSanitization` operation'ı snapshot'ı
`lopdf` ile açar. Main Tauri process PDF parse, stream decode, name-tree/action veya
object-graph traversal yapmaz. Credential gerekiyorsa yalnız session-job secure
credential context'inden worker request exchange'e taşınır; Debug gösterimi redacted,
terminal cleanup generic job lifecycle ile aynıdır. `encrypted` gerçeği credential
presence'tan değil parser'ın input `/Encrypt` durumundan alınır.

Production scanner visited indirect set ve global node budget ile cycle-safe çalışır;
direct dictionary/array/stream dictionary/reference yapılarını izler. Traversal depth,
object/node, per-array ve per-dictionary limitleri; action-chain için ayrı depth/action
budget; name-tree `/Kids`, `/Names`, `/Limits` için ayrı depth/shape budget vardır.
Object/action/name-tree ve stream-dictionary geçişlerinde cancellation checkpoint
bulunur. Runtime cancellation/timeout authority child process'i kill+wait ederek pipe/
registry cleanup'i mevcut session-job semantiğiyle tamamlar. Limit, malformed critical
graph, unresolved reference, unknown action veya incomplete mandatory scan partial
success üretmez; typed `SANITIZATION_*_LIMIT_EXCEEDED` veya
`SANITIZATION_INSPECTION_FAILED` failure ile fail-closed biter.

Canonical kontrol authority'si
`src/features/sanitization/sanitization-controls-v1.json` dosyasıdır. Rust bu manifesti
compile-time include eder; frontend aynı manifesti import eder. Her kontrol inspection
support ile future removal support'u ayrı taşır. Örneğin XFA, optional content,
RichMedia/rendition, encryption ve signature yapıları incelenebilir fakat mevcut
minimum profile için removal unsupported/unverified ve blocking'dir.

Implemented inspection kapsamı:

- Catalog `/OpenAction`, Catalog/Page/Annotation/Field/Widget `/A` ve `/AA`, bounded
  `/Next`; JavaScript, Launch, URI, GoToR, GoToE, SubmitForm, ImportData ve Rendition
  ayrımı. Local GoTo ve destination-shaped OpenAction remote/JavaScript sayılmaz.
- Branch-specific `/Names /JavaScript` ve `/Names /EmbeddedFiles` name-tree taraması;
  `/Dests` sibling'i active content sayılmaz.
- FileSpec, `/EF`, `/RF`, EmbeddedFile, Catalog/Page/Annotation/Structure `/AF`,
  FileAttachment ve Collection presence. Shared indirect FileSpec object identity ile
  deduplicate edilir; file name veya payload rapora konmaz.
- AcroForm/Fields, field/widget actions, `/CO`, XFA; RichMedia, Screen, Movie, Sound,
  3D ve Rendition; `/OCProperties`, OCG, OCMD ve annotation `/OC` presence.
- Trailer Info/ID; Catalog/Page/object Metadata ve PieceInfo/LastModified facts;
  metadata values rapora konmaz.
- Signature field/dictionary, ByteRange, DocMDP ve FieldMDP structural facts. Bu bir
  cryptographic signature validation değildir.
- Actual encryption, parser-observed trailer `/Prev`, ObjStm/XRef structural facts ve
  bounded page count.

Worker sonucu versioned ve strict `SanitizationInspectionReport` üretir:
`schemaVersion`, `inspectionVersion`, `policySchemaVersion`, `documentFacts`, bütün
canonical `controls`, `blockingFindings`, `warnings` ve `limits`. Rust worker üretimden
sonra ve main-process routing öncesinde exact versions/control set/unique IDs/manifest
parity/count/presence/blocking/cross-field/report-byte invariantlarını doğrular. Report
en fazla 1 MiB, 10.000 summary finding, kontrol başına 8 representative evidence,
128 blocker ve 256 warning taşır. JavaScript source, XMP/Info values, embedded bytes,
credential, raw stream veya arbitrary file names yoktur.

Frontend `startSanitizationInspection` canonical `runSessionJob` wrapper'ını ve strict
runtime normalizer'ı kullanır. Svelte 5 Runes store yalnız `idle`, `inspecting`,
`review`, `failed`, `cancelled` lifecycle'ını taşır. Controller session ID, content
generation, job ID ve local revision token'ın tamamını capture eder; document/session/
generation değişimi, yeni inspection veya destroy eski sonucu discard eder ve active
job'ı best-effort cancel eder. App yalnız composition ve context reconciliation yapar.
`SanitizationInspectionPanel.svelte` read-only facts/control/blocker sunumudur; payload
göstermez, destructive control, acknowledgement, Save veya sanitize action sunmaz.
Feature gate kapalı olduğundan production kullanıcı akışı runnable değildir.

Fixture kabulü clean-minimal, metadata paths, JavaScript/name-tree/OpenAction
destination, AA/annotation/form actions, nested/cyclic Next ve network actions,
EmbeddedFiles/EF/RF/AF/FileAttachment/Collection, XFA, optional content/multimedia,
incremental revision, object/xref stream, encryption ve signature corpus'unu kapsar.
Known limitation: inspection bütün PDF aktif/gizli içerik evrenini kapsama garantisi
vermez; private application data experimental, conformance validation kapsam dışı,
signature cryptographic validity incelenmez ve unsupported kategoriler execution için
blocker kalır.
