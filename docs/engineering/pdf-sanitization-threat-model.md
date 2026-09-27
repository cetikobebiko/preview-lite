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
| `document_javascript` | Catalog `/OpenAction`, `/AA`, `/Names /JavaScript`; `/S /JavaScript`; `/JS` literal, hex, stream, indirect; `/Next` | Direct/indirect ve payload biçimleri ayrışır; destination-array negatif kontrolü var | Targeted action/name-tree edge; bütün `/Names` silinmez | Evet | Path/type, string, decoded stream, digest, raw ek scan | Yok | Automation durur | `supported_transform_core` | Independent verification block | JS, destination ve combined Dests/JavaScript/EmbeddedFiles sibling tests |
| `additional_actions` | Catalog/Page `/AA`; Page `/O` `/C`; annotation/widget/field `/A` `/AA`; field `/K /V /C /F`, `/CO` | Key ve action type direct/indirect görülür | Yalnız action entries; form field korunabilir | Detached action için | Independent action graph rescan | Yok | Page/form events durur | `supported_transform_core_with_functional_change` | Independent verification block | JS fixture, bounded walker |
| `annotation_actions` | Annotation/Widget/Link `/A`, `/AA` | Annotation varlığı action'dan ayrı | Link korunup action kaldırılabilir; payload cleanup değildir | Evet | Annotation retained + forbidden action absent | Genelde yok | Link/automation durur | `supported_transform_core_with_functional_change` | Independent verification block | JS/network fixtures |
| `form_actions` | `/AcroForm`, field/widget `/A` `/AA`, `/CO` | Keystroke/validate/calculate/format fixture'da | Form action removal, all-fields removal değildir | Evet | Field inventory + action rescan | Yok | Validation/calculation durur | `supported_transform_core_with_functional_change` | Independent verification block | JS fixture |
| `all_form_fields` | `/Fields`, Widget/AP/inheritance | Inventory mümkün | Appearance/inheritance güvenliği yok | Kapsamlı | Yok | Yüksek | Form kaybolur | `unsupported` | Minimum blocker değil; control execute edemez | Decision |
| `launch_actions` | `/S /Launch`, `/F`, `/Win`, nested `/Next` | Nested/cycle içinde bulunur | Action edge targeted kaldırılabilir | Action/FileSpec | Forbidden type + reachability | Yok | Local launch durur | `supported_transform_core_with_functional_change` | Independent verification block | network fixture |
| `uri_actions` | `/S /URI`, Link `/A` | JS'den ayrı | URI action kalkar, Link kalabilir | Evet | `/S /URI` absent | Yok | Web link durur | `supported_transform_core_with_functional_change` | Independent verification block | network fixture |
| `remote_navigation_actions` | `/GoToR`, `/GoToE`, file spec, destination; local `/GoTo` ayrı | Remote/local ayrımı korunur | Targeted action edge | Evet | Type + external FileSpec scan | Yok | Remote nav durur | `supported_transform_core_with_functional_change` | Independent verification block | network fixture |
| `form_submission_actions` | `/SubmitForm`, `/ImportData`, URL/FileSpec | Ayrı türler deterministic | Targeted action edge | Evet | Forbidden type rescan | Yok | Submit/import durur | `supported_transform_core_with_functional_change` | Independent verification block | network fixture |
| `rendition_actions` | `/Rendition`, Screen/media clip | Üst type görülür | Exhaustive asset graph yok | Karmaşık | Yok | Olası | Media durur | `unsupported` | Minimum blocker | unsupported fixture |
| `embedded_files` | `/Names /EmbeddedFiles`, FileSpec, `/EF`, `/RF`, `/EmbeddedFile` | Branch/FileSpec/streams ayrı; combined sibling fixture aynı `/Names` içindedir | Bütün owner edges kaldırılır; Dests/JavaScript sibling'leri korunur | Zorunlu | Path/graph/decoded digest/raw ek scan | Attachment UI | Ekler kaybolur | `supported_transform_core_with_functional_change` | Independent verification block | AF partial/complete prune ve combined-name-tree tests |
| `associated_files` | Catalog/Page/Annotation/Structure `/AF` | Dört owner ayrı; Structure `/AF` direct ve indirect FileSpec çözer | Her owner edge ayrı; shared FileSpec object-ID dedup karakterize | Zorunlu | No reachable `/AF`/FileSpec/payload | Olası | Association kaybolur | `supported_transform_core_with_functional_change` | Independent verification block | `structure_element_af_resolves_direct_and_indirect_filespecs_without_duplicate_inflation`; AF complete-prune test |
| `file_attachment_annotations` | `/Subtype /FileAttachment`, `/FS`, `/AF` | Subtype/FileSpec görülür | Subtype-specific removal; all-annotation değil | Zorunlu | Subtype/path/payload scan | İkon kaybolur | Attachment erişimi kaybolur | `supported_transform_core_with_visual_change` | Independent verification block | AF fixture |
| `portfolio_collection` | Catalog `/Collection`, embedded name tree | Ayrı görülür | Embedded control ile birlikte | Payload için | Collection + embedded scan | Olası | Portfolio UI kaybolur | `supported_with_functional_change_candidate` | Embedded control ile block | AF fixture |
| `document_info` | Trailer `/Info` | Ayrı path/string | Trailer edge removal | Evet | Trailer + digest scan | Yok | Properties kaybolur | `supported_transform_core` | Independent verification block | metadata fixture |
| `xmp_metadata` | Catalog `/Metadata` stream | Path/type/decoded stream | Targeted owner edge | Zorunlu | Path + digest | Yok | XMP kaybolur | `supported_transform_core` | Independent verification block | metadata fixture |
| `page_metadata` | Page `/Metadata` | Page tree scan | Targeted page entry | Zorunlu | Every-page rescan | Yok | Page properties kaybolur | `supported_transform_core` | Independent verification block | metadata fixture |
| `object_metadata` | Image/Form XObject ve diğer `/Metadata` refs | Image fixture + generic bounded walker | Targeted owner entry | Zorunlu | All-object path + digest | Yok | Object properties kaybolur | `supported_transform_core` | Independent verification block | metadata/object-stream fixtures |
| `private_application_data` | `/PieceInfo`, `/LastModified`, app-private dictionaries | Bilinen keys | Bilinen allowlist paths | Olası | Universe exhaustive değil | Yok | Round-trip data kaybı | `experimental_candidate` | Minimum dışında; seçilemez | metadata fixture |
| `document_identifier` | Trailer `/ID` | Ayrı fact | Preserve/replace/remove teknik olarak mümkün | Hayır | Equality/absence | Yok | Identity/cache semantics | `supported_candidate` | Policy mismatch block | ID writer tests |
| `embedded_file_metadata` | FileSpec `/F`, `/UF`, `/Desc`, `/Params`, dates | Ayrı görülür | Embedded control ile kaldırılır | Evet | FileSpec/payload absent | Yok | Name/desc kaybolur | `supported_with_functional_change_candidate` | Embedded control ile block | AF fixture |
| `xfa` | `/AcroForm /XFA` single indirect stream ve name/stream array refs | Single-stream ve array fixture'larında presence, resolution ve reachability görülür | Controlled fixture edge+prune streams çıkarır; dynamic/script graph exhaustive değil | Zorunlu | Genel verifier yok | Yüksek | Dynamic form kırılır | `unsupported` | Minimum blocker | `xfa_single_reference_resolves_stream_and_prunes_after_edge_removal`; `xfa_array_references_resolve_streams_and_prune_after_edge_removal` |
| `rich_media` | RichMedia, Screen, Movie, Sound, 3D, Rendition, assets/external refs | Üst subtype ve controlled asset | Tüm relations enumerate edilmedi | Zorunlu | Yok | Yüksek | Multimedia durur | `unsupported` | Minimum blocker | unsupported fixture |
| `optional_content` | `/OCProperties`, OCG, OCMD, marked-content refs, visibility, annotation `/OC` | OCG ve OCMD type'ları, resource refs ve layer-associated annotation ayrı görülür | `/OCProperties` removal marked content, OCMD ve annotation `/OC` izlerini kaldırmaz | Dictionary prune yetersiz | Görsel/semantic verifier yok | Yüksek | Layer semantics bozulur | `unsupported` | Minimum blocker | `optional_content_ocg_ocmd_and_annotation_oc_are_independently_detectable`; removal-insufficiency test |
| `all_annotations` | `/Annots`, subtype/AP/payload | Inventory mümkün | Tüm subtype safety yok | Kapsamlı | Yok | Yüksek | Yorum/form/media kaybı | `unsupported` | Minimum blocker değil; active subtype kendi blocker'ı | subtype corpus |
| `incremental_revisions` | `/Prev`, multiple classic xref, real xref-stream chain, hybrid | Raw+parser facts; latest graph ve physical previous payload ayrışır | Explicit cleanup+prune+full save | Zorunlu | Single EOF/no history/old digest/output-local decoded xref/reopen | Yok | History gider | `supported_transform_core` | Independent verification block | classic ve xref-stream-chain full-rewrite tests |
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
Decoded payload incelemesi gerektiren JavaScript stream'leri stream başına 16 MiB ve
inspection job toplamında 128 MiB decoded-byte budget'a tabidir. `limits.decodedBytes`
mandatory decode'larda gerçekten tüketilen byte toplamıdır; checked arithmetic overflow,
stream başına +1 ve toplam +1 durumları
`SANITIZATION_DECODE_LIMIT_EXCEEDED` ile partial report olmadan fail-closed biter. Salt
dictionary presence ile güvenilir saptanan metadata, attachment, XFA ve benzeri stream'ler
sırf accounting için decode edilmez.

`lopdf` 0.34 stream API'si whole-buffer decode döndürdüğünden bu limit parser'ın
decode sırasındaki ilk allocation'ını preempt etmez. Decoded sonuç döner dönmez
per-stream ve total budget enforce edilir; oversized payload finding veya başarı raporu
üretemez. Parser-level DoS sınırı ayrı ephemeral process, worker address-space limiti
ve parent timeout/kill/wait birlikteliğidir; bounded/pre-allocation decode garantisi yoktur.

Production sanitization cancellation cooperative worker IPC kullanmaz. Scanner'ın
checkpoint-capable seam'i yalnız deterministic unit test authority'sidir; production
cancellation authority parent-owned child process registry'sinin token kontrolü,
kill+wait/reap akışıdır. User cancellation canonical `Cancelled` outcome'a; timeout ve
typed worker crash `Failed` outcome'a map edilir. Sanitization'a özgü real-process kabul
testleri traversal/decode sırasında cancellation, crash, timeout, wrong-session,
stale-generation, session-close ve wrong-owner cancellation davranışlarını; private job
directory, child registry ve credential context cleanup'ini doğrular. Runtime
cancellation/timeout authority child process'i kill+wait ederek pipe/registry cleanup'i
mevcut session-job semantiğiyle tamamlar. Limit, malformed critical
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
parity/count/presence/blocking/cross-field/report-byte invariantlarını doğrular. Aynı
`controlId` için birden fazla blocker reddedilir; blocker matching control'ün
`present == true`, `findingCount > 0` sonucunu gerektirir ve
`0 < evidenceCount <= findingCount` olmalıdır. Rust validator ve frontend normalizer aynı
malformed wire vakalarını reddeder. Report
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

## 22. 8C.3 Transform Implementation Status

8C.3 private transform core uygulanmıştır; bu bir kullanıcıya yayımlama veya verified
sanitization sonucu değildir. `sanitization-policy-v1` strict execution contract'ı
`schemaVersion`, `profile`, canonical `controls`, strict `acknowledgements` ve
`expectedInspectionVersion` taşır. `verified_limited_v1` registry'de
`supported*` işaretli bütün mandatory control'leri ister;
`custom_supported_v1` yalnız aynı allowlist'i kabul eder. Bütün profiller explicit
`incremental_revisions` ister. Legacy broad boolean payload, unknown/duplicate/
unsupported control, excessive control count ve unknown serde fields fail-closed'dur.

Canonical `sanitization-controls/1` registry'de transform desteği verilen controls:
`document_javascript`, `additional_actions`, `annotation_actions`, `form_actions`,
`launch_actions`, `uri_actions`, `remote_navigation_actions`,
`form_submission_actions`, `embedded_files`, `associated_files`,
`file_attachment_annotations`, `document_info`, `xmp_metadata`, `page_metadata`,
`object_metadata` ve `incremental_revisions`. Candidate/experimental/unverified/
unsupported durumlar executable allowlist'e girmez. Özellikle XFA, Rendition/
RichMedia, optional content, encryption, all annotations/forms, private application
data, portfolio, document identifier ve embedded-file metadata ayrı production
mutation iddiası taşımaz. Object-metadata scanner kapsamı nedeniyle custom policy'de
`object_metadata`, catalog ve page scope controls ile birlikte istenir. `/ID` writer
tarafından preserve edilir; regenerate/remove policy uygulanmaz.

Internal `SanitizePdf` ephemeral worker operation'ı yalnız immutable `snapshot.pdf`,
session-job credential context, captured content generation, policy, expected input
digest, expected inspection digest ve sabit `sanitized-transform.pdf` adını alır.
External target/path yoktur. Worker current snapshot üzerinde inspection'ı yeniden
üretir; inspection/policy/control-registry version, snapshot SHA-256 ve canonical
inspection-report SHA-256 eşleşmeden mutation başlamaz. Encrypted input
`SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED`; unsupported blocker
`SANITIZATION_UNSUPPORTED_CONTENT`; signed input acknowledgement yoksa
`SANITIZATION_SIGNATURE_ACKNOWLEDGEMENT_REQUIRED` üretir. Acknowledged signed output
signature invalidation ve certification-loss semantiğini açık warning/facts ile taşır;
signature preservation iddiası yoktur.

Mutation layer read-only scanner'dan ayrıdır. Action edges direct/indirect dictionary,
array ve cycle-safe `/Next` graph'ında type/origin bazlı kaldırılır. Local `GoTo` ve
destination-shaped `/OpenAction` korunur. `/A`, `/AA` ve `/Next` boş/broken container
bırakmadan hedeflenir; field/widget dictionary'leri ve values korunur. `/Names`
mutasyonu yalnız requested `JavaScript` veya `EmbeddedFiles` branch'ini kaldırır;
`Dests` ve unrequested sibling'ler korunur. Embedded-file policy name-tree, `/AF`,
FileSpec `/EF`/`/RF` ve FileAttachment owner edges'ini birlikte ele alır; shared veya
surviving reachability lopdf pruning authority'sine bırakılır. Metadata policy Trailer
`/Info` ve catalog/page/object `/Metadata` owner edges'ini scope bazlı kaldırır.

Mutation öncesi requested inspection inventory authoritative reporttan bağlanır;
kaldırılan JavaScript, XMP/metadata ve embedded payload için decoded/plain bytes
SHA-256 fingerprint'i bounded evidence olarak tutulur. Raw payload reporta girmez.
Fingerprint için açılan her `Object::Stream`, 8C.2'nin canonical 16 MiB
`MAX_SANITIZATION_DECODED_STREAM_BYTES` limiti ve 128 MiB
`MAX_SANITIZATION_TOTAL_DECODED_BYTES` toplam bütçesi ile account edilir. Sıra
decode sonucu → stream-başı limit → checked total addition → total limit →
fingerprint'tir; stream-başı +1, total +1 veya arithmetic overflow
`SANITIZATION_DECODE_LIMIT_EXCEEDED` ile report/artifact success olmadan
fail-closed olur. `Object::String` mevcut total payload accounting semantiğini
korur; decoded-stream olarak yeniden sınıflandırılmaz. `lopdf` whole-buffer
decode API verdiği için bu transform kontrolü de post-decode enforcement'tır:
ilk allocation'ı preempt ettiği iddia edilmez, sonuç döner dönmez fingerprint
öncesi reddedilir.
Mutation sonunda `/Prev` ve `/XRefStm` temizlenir, `prune_objects()` ile unreachable
graph çıkarılır, bütün surviving references için dangling-ref self-check yapılır ve
PDF yeni private staging dosyasına baştan serialize edilir. Reopen, non-empty/size
limit, regular-file/no-symlink, fixed-name, job-directory containment ve output digest
kontrollerinden sonra artifact private job path'ine promote edilir. Main orchestration
artifact SHA-256 ve size'ı worker reportundan bağımsız tekrar hesaplar.

`sanitization-transform/1` report; policy/registry/inspection versions, profile,
input/output/inspection digests, content generation, requested-control
`removed_candidate | not_found` sonuçları, bounded path/object/fingerprint evidence,
full-rewrite structural facts, signature warnings ve kesin
`transformed_unverified` status'u taşır. `verifiedRemoved` veya publication assurance
alanı yoktur. Worker structural validator kendi içindeki version/digest/control
invariantlarını denetler; parent ayrıca report'u original validated request'e
bağlar: exact policy schema version, profile, expected inspection version,
input digest, inspection digest ve requested control seti eşit olmalıdır.
Eksik, extra, duplicate veya different supported control; wrong profile ya da
wrong request digest parent boundary'de `SANITIZATION_TRANSFORM_REPORT_MISMATCH`
ile reddedilir. Bu correlation `verified_limited_v1` mandatory setini veya
`custom_supported_v1` exact custom setini gevşetmez. Parent-owned worker timeout/cancel kill+wait/reap mekanizması transform,
pruning ve serialization sırasında process'i herhangi bir noktada sonlandırabilir;
failure/cancel generic private job cleanup ile incomplete artifact, request/response,
credential ve child registry state'ini temizler. Main Tauri process lopdf parse,
stream decode, mutation, pruning veya serialization yapmaz.

Public `sanitize_pdf_document` hâlâ parse/I/O yapmadan `FEATURE_UNAVAILABLE` döndürür;
internal transform/publication core Tauri invoke handler'a kayıtlı değildir. Save dialog ve
frontend policy editor/execution workflow yoktur; 8C.5 publication yalnız internal/testable
backend authority'sidir.
Private artifact yalnız active `run_snapshot_job` operation closure süresince
geçerlidir: transform worker → parent report/artifact validation → independent
verifier aynı private artifact'ı tüketir → terminal cleanup. Successful `Completed`
outcome transform artifact'ını bir sonraki ayrı job'a devretmez; terminal cleanup job
directory'yi siler. Bu nedenle 8C.4 verifier aynı snapshot-job lifecycle içinde
orchestrate edilmelidir; persistent temp retention, frontend path exposure veya cleanup
devri yapılamaz. Feature registry kesin olarak disabled kalır.

## 23. 8C.4 Independent Verification Implementation Status

8C.4 bağımsız verification uygulanmıştır. `SanitizePdf` tamamlanıp parent report,
regular-file, containment, size ve SHA-256 doğrulamasını geçtikten hemen sonra parent
artifact digest'ini yeniden hesaplar. Ancak bu pre-verification digest gate geçerse ayrı
bir ephemeral process `VerifySanitizedPdf` operation'ıyla başlatılır. Verifier transform
builder'ı veya worker'ın in-memory state'ini çağırmaz; sabit
`sanitized-transform.pdf` adını private job directory'den kendisi türetir, symlink/
escape/non-file/empty/oversize durumlarını reddeder ve artifact'ı mutate etmez.
Original/working/target path veya publication authority almaz. Global snapshot-job
request'inin immutable snapshot alanı verifier semantiğinde kullanılmaz.

Strict `sanitization-verification/1` report policy/registry/inspection/verifier
versions, profile, input/transform digest'leri, requested-control results ve typed
`passed | failed | skipped_with_reason` check'leri taşır. Mandatory checks:
`fullRewrite`, `incrementalHistoryRemoved`, `objectGraphScan`, `removedPayloadScan`,
`parserReopen`, `pdfiumReopen`, `pageTreeValid`, `pageCountValid`,
`pageDimensionsValid`, `preservationChecks`, bounded `renderSmoke` ve
`artifactUnchanged`'dır. Failed, skipped veya malformed mandatory check;
blocker; unresolved mandatory finding; eksik/duplicate/unknown control ya da failed
control sonucu `isVerified == true` ile birlikte kabul edilemez. Worker ve parent bu
biconditional invariantı ayrı ayrı doğrular. Parent ayrıca transform report digest ==
verification report digest == post-verification actual artifact digest eşitliğini
yeniden kurar; wrong schema/verifier/policy/profile/input/transform digest ve control
seti fail-closed reddedilir.

Verifier output'u fresh lopdf parser instance ve read-only 8C.2 scanner ile yeniden
açar. Requested controls için fresh finding count sıfır olmalıdır; action scanner
JavaScript, Launch, URI, GoToR/GoToE, SubmitForm/ImportData, origin-specific `/A`/`/AA`
ve bounded `/Next` zincirlerini tekrar gezer. Embedded-file verification Names/
EmbeddedFiles, FileSpec EF/RF, EmbeddedFile streams, AF ve FileAttachment semantiğini
8C.2 scanner ile tarar. Metadata sonucu policy'nin document Info, catalog/page/object
Metadata scope'larıyla birebir bağlıdır. Transform report'un removed count veya
self-check sonucu authority değildir.

Transform evidence'taki JavaScript/string, decoded metadata ve embedded payload
SHA-256 fingerprint'leri verifier tarafından output'taki nested string object'ler ve
decoded/relevant stream'ler üzerinde tekrar aranır. Scan direct/indirect arrays,
dictionaries, streams ve orphan serialization dahil bütün parsed objects için
cycle-safe/bounded çalışır. Raw bytes yalnız full-rewrite yardımcı kanıtıdır; payload
absence sonucu object graph + decoded payload scan'den gelir. Report veya error'lara
JavaScript source, metadata values, embedded bytes, credential ya da arbitrary payload
konmaz; yalnız safe code/control/count/path/object structural evidence taşınır.

Full-rewrite verification active `/Prev` ve `/XRefStm` yokluğunu; tek PDF header,
`startxref` ve EOF serialization'ını; resolvable current graph'ı; dangling critical
reference yokluğunu; current xref-stream infrastructure dışında `prune_objects()` ile
orphan/unreachable indirect object yokluğunu birlikte ister. Bu `%EOF` sayımına
indirgenmez. Removed payload fingerprint'leri eski
revision veya unreachable object içinde yeniden bulunursa verification başarısızdır.
Adversarial acceptance fake transform-success + reintroduced JS/fingerprint,
reintroduced orphan EmbeddedFile, wrong digest, appended old-revision marker, dangling
reference ve page-count corruption vakalarını reddeder.

Transform worker input graph'ından expected page count, CropBox/MediaBox dimensions,
rotation, decoded page-content digest, named-destination count, local GoTo count,
top-level form-field count ve outline presence facts üretir. Verifier bunları output
graph'den bağımsız yeniden gözlemler. Named destinations, fields, outline ve page
content daima korunmalıdır; local GoTo origin-level action removal istenmedikçe
korunmalıdır. PDFium output'u ayrı açar; page geometry pozitif/finite olmalı ve first,
last, varsa middle page 64-pixel target width ile bounded render smoke geçmelidir.
PDFium parsing/rendering verifier worker içindedir, ana Tauri process PDF açmaz.

Verifier 8C.2'nin object, traversal depth, raw bytes ve total decoded bytes limitlerini;
ayrıca fingerprint/page/report collection limitlerini reuse eder. Limit nedeniyle
mandatory observation tamamlanamazsa success üretmez. Parent-owned cancellation her
worker phase'inde child kill+wait/reap authority'sidir; deterministic acceptance seam'i
verifier traversal sırasında cancel, crash ve timeout davranışlarını kanıtlar. Crash,
timeout, cancel, mandatory skip ve PDFium reopen failure publication üretmez, verified
artifact kabul etmez, exchange/private job directory terminal lifecycle ile temizlenir
ve session ID/content generation/state revision/dirty/working path/save target değişmez.

### 23.1 8C.4.1 verifier closure

Fresh output inspection'daki canonical `blockingFindings` verification authority'sidir:
requested-control sonucu temiz olsa bile XFA, optional content, RichMedia/rendition,
encryption veya registry'nin başka blocking kategorisi bulunursa typed mandatory
unresolved finding ve verification blocker üretilir; `isVerified` false olur. Bu
taxonomy verifier içinde tekrar tanımlanmaz, registry/policy değerlendirmesi reuse
edilir. Acknowledged signature invalidation yalnız transform report'unun bu
invalidation'ı bağladığı imzalı input akışında istisnadır; otherwise reintroduced
signature blocker'dır.

Removed-payload absence proof decoded object/string payload gözlemidir. Encoded/raw
stream bytes yardımcı kanıt olabilir, fakat decoded scan'in yerini alamaz. Taranan
stream decode edilemiyorsa `removedPayloadScan` failed olur; parser reopen passed
olması veya encoded bytes içinde fingerprint bulunmaması success değildir. Decode
accounting central 16 MiB per-stream ve 128 MiB total limitlerini decode → per-stream
check → checked total → total-check → digest sırasıyla uygular. `lopdf` whole-buffer
decode kullandığından bu post-decode enforcement'tır; ilk allocation'ı önlediği iddia
edilmez.

Actual artifact page count, expected input invariantinden bağımsız olarak canonical
10,000 verification-page limitine bağlanır. Limit aşılırsa page geometry traversal ve
PDFium first/last/middle sampling success üretemez. Transform → verifier aynı active
snapshot-job closure içinde kalır. Publication terminal cleanup'ten önce 8C.5'te
eklenecektir; bu aşamada external target, Save dialog veya frontend execution yoktur.

Known gaps: frontend policy/execution/report workflow 8C.6'ya; feature activation 8C.7'ye
aittir. Current source-byte-copying proof
tek serialization, history pointers, unreachable graph, old-revision structural marker
ve removed-payload fingerprints üzerine kuruludur; arbitrary non-sensitive byte-block
semantic equivalence iddiası taşımaz. macOS x86_64, Windows ve Linux runtime verifier
certification çalıştırılmamıştır.

## 24. 8C.5 Atomic Publication Implementation Status

8C.5 digest-bound atomic publication backend'i uygulanmıştır. Internal
`execute_document_sanitization_core` tek immutable snapshot-job lifecycle'ında fresh
inspection authority → `SanitizePdf` worker → ayrı `VerifySanitizedPdf` worker → main-process
publication zincirini çalıştırır. Bu core Tauri invoke handler değildir; production Save
dialog veya frontend execution callsite'ı açmaz. Legacy `sanitize_pdf_document` kesin olarak
`FEATURE_UNAVAILABLE`, registry `sanitization.status == "disabled"` ve `enabled == false`
kalır.

Target preflight, Save/Save As'ın paired write lease'ine karşı generic verified-artifact
export read lease'i altında yapılır. Original source, current working PDF ve bunların canonical
namespace/filesystem-identity alias'ları; session private root; jobs root; directory;
symlink/reparse-point; dangling symlink; unsupported multiple-hard-link target; missing veya
unusable parent reddedilir. Missing target expectation canonical namespace'e; existing target
expectation canonical namespace + filesystem identity + baseline SHA-256'a bağlanır. Shared
path lock altında replacement'tan hemen önce expectation yeniden doğrulanır. Missing→created,
existing→modified, same-digest/different-identity replacement ve deleted/recreated yarışları
`SANITIZATION_TARGET_CONFLICT` ile external writer bytes'ını korur.

Worker artifact yolu frontend veya caller girdisi değildir. Parent yalnız private job root
altındaki canonical `sanitized-transform.pdf` regular file'ını kabul eder; symlink/escape veya
başka isim reddedilir. Main process artifact'ı lopdf/PDFium ile açmaz ve semantic verification
tekrarlamaz. Strict worker report validator schema/version, policy/profile, requested controls,
input/transform digest, bütün mandatory checks, blockers, unresolved mandatory findings ve
`isVerified` biconditional'ını yeniden doğrular. Publication öncesi invariant:

`transform.outputDigest == verification.transformDigest == actualArtifactDigest`.

Shared `document_io` verified-artifact primitive artifact'ı kopyadan hemen önce yeniden hash'ler,
target parent içinde adjacent staged temp oluşturur, temp'i fsync eder ve staged SHA-256'yı verifier
digest'iyle karşılaştırır. Existing target pre-delete edilmez. Platform-specific atomic replace,
post-commit filesystem identity, post-commit target SHA-256 ve parent-directory durability
kontrolleri redaction ile aynı generic primitive'dir; sanitization modülünde platform kodu yoktur.
Successful report yalnız `new_committed | new_committed_durability_unconfirmed` kabul eder ve
`published == true` ancak verifier PASS ve published target digest exact chain ile eşleştiğinde
mümkündür.

Pre-commit failure `original_preserved`/`prepared` disk truth'unu; committed fakat durability
confirmation başarısızlığı `new_committed_durability_unconfirmed` sonucunu korur. Filesystem
sonucu kesin değilse `indeterminate`; recovery artifact gerektiğinde `recovery_required` ile
`recoveryPath` ve `replacementDigest` terminal failure metadata'sında taşınır. Generic job cleanup
ordinary staged/job/snapshot/exchange/credential/child state'ini temizler, recovery artifact'ı
silmez. `sanitization-test-support` compile-time seam'i temp create/copy/sync, staged corruption,
replace, postcommit identity/digest, parent durability ve recovery-required sonuçlarını test eder;
environment-variable veya default/release user switch'i yoktur.

Cancellation checkpoint atomic publication başlamadan hemen öncedir. O noktadan önce cancel
target'ı değiştirmez. Atomic boundary başladıktan sonra cooperative cancellation filesystem
transaction'ı bölmez; committed bytes varsa `Completed` ve typed publication report döner,
`cancelRequested` metadata'sı korunabilir. Export success/failure/conflict/cancel session ID,
content generation, state revision, dirty, working path, save target veya active document'i
değiştirmez ve output'u auto-open etmez. Deterministic Save As concurrency testi publication read
lease'i bırakılmadan persistence mutation'ın ilerleyemediğini doğrular.

Feature matrix: **Inspection implemented; Transform implemented; Independent verification
implemented; Atomic publication implemented; Frontend execution workflow not yet complete;
Feature disabled.** Runtime acceptance macOS arm64 üzerinde missing/existing happy path, exact
four-way digest chain, artifact/staged/target TOCTOU, path/link safety, atomic fault states,
cancellation boundary, Save As lease, cleanup ve session neutrality için PASS'tir. macOS x86_64,
Windows ve Linux runtime publication certification çalıştırılmamıştır.

## 25. 8C.6 Frontend Workflow Status

8C.6 canonical frontend zincirini tamamlar:

```text
featureRuntime.sanitization
  → sanitizationController
  → sanitizationState
  → SanitizationPanel
  → platform Save adapter
  → startSanitizationExecution
  → execute_document_sanitization
```

Workflow açıldığında current inspection yoksa ilk adım inspection'dır. Panel safe
document facts, finding count, inspection/removal support, beklenen görsel/işlevsel
etki ve blocker reason gösterir; raw JavaScript, XMP, embedded bytes veya credential
state/UI/log'a taşınmaz. Inspection metni yalnız desteklenen yapıları taradığını ve
bütün active/hidden content'i bulma garantisi vermediğini açıkça söyler.

`verified_limited_v1` control seti frontend tarafından yeniden icat edilmez; versioned
control manifest'in bütün production-supported kontrolleridir. `custom_supported_v1`
yalnız aynı supported subset'i seçebilir. Unsupported/unverified/experimental controls
disabled'dır. XFA, RichMedia/rendition, optional content ve encryption blocker'ları
Save dialog ve execution'ı engeller; acknowledgement bypass değildir. Signed input
yalnız ayrı, default-false signature invalidation acknowledgement ile ilerler. Bu onay
imzanın “kaldırıldığını” değil full rewrite nedeniyle imza/certification durumunun
geçersizleşebileceğini ifade eder. Standard conformance korunması doğrulanmadığı için
panel PDF/A, PDF/X, PDF/UA, certification veya linearization korunur iddiası taşımaz.

Profile/control/acknowledgement değişimi workflow revision'ı artırır ve önceki
execution result/request authority'sini invalid eder. Save dialog yalnız current
session/generation inspection, geçerli supported policy, blocker yokluğu ve gerekli
onaylardan sonra platform adapter üzerinden açılır. Cancel edilen Save dialog backend
job başlatmaz. Execution request session ID, content generation, inspection version,
profile, exact controls, acknowledgements ve workflow revision içeren immutable
snapshot'tır. Backend current immutable snapshot üzerinde fresh inspection ve policy
revalidation yaptığı için frontend raporu security authority değildir.

Inspection ve execution job handle'ları ayrıdır. Session/generation/document type
değişikliği veya controller destroy active handle'ları best-effort cancel eder ve stale
ownership guard sonucu reddeder. Kullanıcı cancel istediğinde execution token
geçersizleştirilmez: backend publication commit sınırını geçmiş ve valid published
report dönmüşse UI completed disk truth'u korur. Successful export active source
document/session/generation/dirty/revision state'ini değiştirmez ve output auto-open
edilmez.

Result presentation yalnız backend assurance'a göre `verified_limited` ile
`verified_custom` wording'ini ayırır; requested-but-absent control `not_found` olarak
kalır. Verification failure, target conflict, durability-unconfirmed success,
`recovery_required` metadata ve `indeterminate` disk truth ayrı gösterilir. Legacy
frontend broad boolean contract, implicit `_sanitized.pdf` hedefi ve source auto-replace
akışı kaldırılmıştır. Legacy Rust `sanitize_pdf_document` fail-closed kalır; canonical
versioned command `execute_document_sanitization`dır.

Bu aşama activation değildir. `sanitization.status == "disabled"` ve
`sanitization.enabled == false` korunur. macOS arm64 backend runtime acceptance PASS;
macOS x86_64, Windows ve Linux runtime certification 8C.7 blocker'ıdır.

## 26. 8C-7 Final Security Audit, Platform Certification ve Activation Kararı

8C-7 final güvenlik denetimi, platform sertifikasyon değerlendirmesi ve aktivasyon kararı tamamlanmıştır.

### 26.1 Bütünlük ve Güvenlik Denetimi Özeti

1. **Mimarlık Zinciri:** Active PDF session → immutable snapshot → sanitization inspection worker → authoritative inspection report → strict versioned policy → transform worker → targeted object-tree mutation → object pruning → private full-rewrite artifact → independent verification worker → object/payload/full-rewrite verification → digest-bound atomic publication → structured execution report → frontend verified workflow zincirinin tamamının mevcut olduğu ve çalıştığı doğrulanmıştır.
2. **Main-Process Güvenlik Sınırı:** Ana Tauri process'inin kullanıcı PDF'sini sanitization semantic work için hiçbir şekilde parse etmediği (ör. `lopdf::Document::load`, PDFium parsing, object traversal, action scan, metadata scan vb. yapılmadığı) architecture testleriyle kilitlenmiştir.
3. **Threat Model Parity:** `docs/engineering/pdf-sanitization-threat-model.md` ile canonical `sanitization-controls-v1.json` manifesti ve backend/frontend kodu arasında 1:1 uyum sağlanmıştır.
4. **Supported Controls (PASS):** `document_javascript`, `additional_actions`, `annotation_actions`, `form_actions`, `launch_actions`, `uri_actions`, `remote_navigation_actions`, `form_submission_actions`, `embedded_files`, `associated_files`, `file_attachment_annotations`, `document_info`, `xmp_metadata`, `page_metadata`, `object_metadata` ve `incremental_revisions` kontrollerinin inspection, targeted mutation, pruning, full rewrite, independent verification, frontend mapping ve fixture coverage adımlarının tamamı PASS olarak doğrulanmıştır.
5. **Unsupported Controls (BLOCKER):** XFA, RichMedia/multimedia, optional content (`/OCProperties`), encryption (`SANITIZATION_ENCRYPTED_INPUT_UNSUPPORTED`), `all_annotations` ve `all_form_fields` fail-closed blocker olarak kalmış, sessizce atlanmadıkları ve UI'da yanlış temizleme garantisi vermedikleri doğrulanmıştır.
6. **Legacy Model Temizliği:** Legacy broad boolean contract (`remove_metadata`, `remove_annotations`, `remove_javascript`, `remove_embedded_files`, `remove_hidden_layers`), otomatik `_sanitized.pdf` output türetimi ve source auto-replace akışı tamamen kaldırılmıştır. Legacy `sanitize_pdf_document` komutu `FEATURE_UNAVAILABLE` olarak fail-closed bırakılmıştır.
7. **Bağımsız Verifier Authority:** Transform worker ve verification worker ayrı process'ler olarak ayrılmıştır. Verifier; fresh parser reopen, digest recomputation, object graph scan, removed payload fingerprint scan, full rewrite check ve PDFium reopen/render smoke testlerini zorunlu kılar. Adversarial testlerle fake transform success raporları reddedilmiştir.
8. **Atomik Yayınlama ve Target Güvenliği:** External publication yalnız `verification.isVerified == true` şartıyla yapılmaktadır. Digest zinciri (`transform.outputDigest == verification.transformDigest == publication.publishedDigest == actual target SHA-256`) tam olarak eşleşir. Target preflight; source/working PDF, private job dizini, symlink, hardlink ve dizin hedeflerini reddeder. Target TOCTOU ve staged temp corruption durumlarında orijinal dosya korunmaktadır.
9. **UI Güvenlik Dili ve Seam İzolasyonu:** UI wording dürüsttür ("Ön inceleme yalnız desteklenen PDF yapılarını tarar..."). "Tehdit yok", "PDF güvenli", "Belge tamamen temiz" gibi abartılı ifadeler kesinlikle kullanılmamıştır. Test/debug seam'leri release build'e sızmayacak şekilde `#[cfg(feature = "sanitization-test-support")]` altında izole edilmiştir.

### 26.2 Platform Sertifikasyon Matrisi

| Platform | Runtime Acceptance Durumu | Karar Notu |
|---|---|---|
| macOS arm64 | **PASS** | Full real-process inspection, transform, verifier, publication, target TOCTOU, fault recovery, cancellation ve session neutrality testlerinin tamamı başarıyla geçmiş ve doğrulanmıştır. |
| macOS x86_64 | **NOT RUN** | Target platform üzerinde runtime publication acceptance çalıştırılmamıştır. |
| Windows | **NOT RUN** | Target platform üzerinde runtime publication acceptance çalıştırılmamıştır. |
| Linux | **NOT RUN** | Target platform üzerinde runtime publication acceptance çalıştırılmamıştır. |

### 26.3 Final Aktivasyon Kararı

**Decision: Outcome B — DEFER ACTIVATION**

* **Açıklama:** PDF Sanitization altyapısı, backend güvenlik sınırları, bağımsız verifier, atomik yayınlama zinciri ve frontend workflow'u %100 tamamlanmış, test edilmiş ve mimari olarak doğrulanmıştır. Ancak advertised platform setinin (macOS x86_64, Windows, Linux) runtime sertifikasyon testleri henüz çalıştırılmadığından, tahribat yaratma potansiyeli olan bu güvenlik özelliğinin tüm masaüstü platformlarında genel kullanıma sunulması ertelenmiştir.
* **Feature Registry Durumu:** `sanitization.status = "disabled"`, `sanitization.enabled = false`.
* **Warning Metni:** `"Sanitization inspection, verified execution ve frontend review workflow'u uygulanmıştır; 8C-7 platform/security aktivasyon kararı tamamlanana kadar özellik kapalıdır."`

### 26.4 Final Kapanış Kararı

> **8C COMPLETE — ACTIVATION DEFERRED PENDING PLATFORM CERTIFICATION — PROMPT 9 READY**

