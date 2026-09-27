# AI Endpoint, RAG ve Streaming Güvenlik Mimarisi (Prompt 9A.12 Final Acceptance Closure)

## 1. Mimarinin Amacı ve Sınırları
Bu doküman Antigravity PDF Reader uygulamasında AI endpoint yönetimi, RAG (Retrieval-Augmented Generation) indeksi, VectorStore bellek yaşam döngüsü, HTTP streaming iletişimi ve iptal yönetimi için uygulanan Prompt 9A güvenlik kontrollerini ve kabul kriterlerini belgeler.

## 2. Temel İlkeler ve Mimari Sınırlar (Invariants)
- **Sıfır Yeni Bağımlılık**: Yeni npm paketi, Rust crate'i, Tauri eklentisi veya framework eklenmemiştir (`AGENTS.md`).
- **PDF Motoru İzolasyonu**: PDFium kanonik render/text extraction motorudur. PDF ayrıştırma ana Tauri process'inde veya JS ortamında yapılmaz; DocumentSession ve worker izolasyon sınırları korunur.
- **Kilit Sıralaması (Lock Hierarchy)**: `DocumentSession` durum kilitleri hiçbir zaman `VectorStore` kilit alımı veya giden HTTP I/O operasyonları sırasında tutulamaz.
- **Atomik İndeks Yayımı (Atomic Replacement)**: Model, parça politikası veya endpoint değiştiğinde staging katmanında yeni indeks derlenir; başarılı olana kadar eski geçerli indeks bellekte korunur (I7).

---

## 3. Tam AI HTTP İptal Çağrı Grafiği (Cancellation Call-Graph Audit)

Tüm 12 ağ/await noktası `wait_for_cancellation` ile `tokio::select!` yarışına dahil edilmiştir. Üretim kodundaki stream akışları ve indeksleme pipeline'ları `consume_ai_ndjson_stream` ve `run_index_job_internal` crate-içi yardımcı fonksiyonları üzerinden çalıştırılarak acceptance testleriyle 1:1 üretim denkliği sağlanmıştır:

| Faz # | Operasyon / Fonksiyon | Await / Ağ Noktası | İptal Mekanizması | Pre-9A.11 Durumu | Prompt 9A.12 Doğrulanmış Durumu |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1** | `build_semantic_index` / `run_index_job_internal` | `extract_full_document_text` | Worker Cancellation Channel | Korundu | Worker iptal token'ı ile anında durur |
| **2** | `build_semantic_index` / `run_index_job_internal` | `send_post_json` (`/api/embeddings`) | `wait_for_cancellation` race | Eksikti | Connect/Header fazında anında abort |
| **3** | `build_semantic_index` / `run_index_job_internal` | `fetch_json_body` | `wait_for_cancellation` race | Eksikti | Read-stall fazında anında abort |
| **4** | `build_semantic_index` / `run_index_job_internal` | Chunk loop iteration | Iteration cancel check | Korundu | Döngü anında sonlanır, staging temizlenir |
| **5** | `semantic_search` / `semantic_search_internal` | `send_post_json` (`/api/embeddings`) | `wait_for_cancellation` race | Eksikti | Connect/Header fazında anında abort |
| **6** | `semantic_search` / `semantic_search_internal` | `fetch_json_body` | `wait_for_cancellation` race | Eksikti | Read-stall fazında anında abort |
| **7** | `query_document_rag` / `consume_ai_ndjson_stream` | `send_post_json` (`/api/chat`) | `wait_for_cancellation` race | Eksikti | Connect/Header fazında anında abort |
| **8** | `query_document_rag` / `consume_ai_ndjson_stream` | `response.bytes_stream().next()` | `wait_for_cancellation` race | Kısmi | Akış sırasında anında abort ve terminal event |
| **9** | `query_document_rag` / `consume_ai_ndjson_stream` | Parse & Emit Frame | Idempotent terminal emit | Kısmi | `emit_cancelled` çağrısı idempotent korunur |
| **10** | `summarize_selection_or_page` / `consume_ai_ndjson_stream` | `send_post_json` (`/api/generate`) | `wait_for_cancellation` race | Eksikti | Connect/Header fazında anında abort |
| **11** | `summarize_selection_or_page` / `consume_ai_ndjson_stream` | `response.bytes_stream().next()` | `wait_for_cancellation` race | Kısmi | Akış sırasında anında abort ve terminal event |
| **12** | `summarize_selection_or_page` / `consume_ai_ndjson_stream` | Parse & Emit Frame | Idempotent terminal emit | Kısmi | `emit_cancelled` çağrısı idempotent korunur |

---

## 4. Salt-Okunur Önbellek Doğrulama ve Otoriter Yaşam Döngüsü

### 4.1 Salt-Okunur Doğrulama (M1–M4)
Arama (`semantic_search`) ve RAG sorgularında (`query_document_rag`) oturum indeksi önbellek denetimi kesinlikle **salt-okunurdur (read-only)**. Uyuşmazlık durumunda (`IndexOutdated`):
- `VectorStore` içerisindeki mevcut indeks **asla silinmez, budanmaz veya mutasyona uğratılmaz**.
- Küresel bellek sayacı değişmez.

### 4.2 Otoriter Budama Noktaları
İndeks budama işlemi yalnızca şu 4 otoriter yaşam döngüsü olayında gerçekleşir:
1. **Otoriter İçerik Jenerasyonu Değişimi**: Belge mutasyonu gerçekleştiğinde (`prune_generation`).
2. **Oturum Kapanışı / İmhası**: Belge sekmesi veya oturumu kapatıldığında (`remove_session`).
3. **Endpoint Sıfırlama / Parmak İzi Değişimi / Yeniden Yapılandırma**: Endpoint temizlendiğinde veya `configure_ai_endpoint` ile yeniden yapılandırıldığında muhafazakar invalidation ile tüm vektörler temizlenir (`clear_all`).
4. **Atomik İndeks Değişimi**: Yeni bir indeks başarıyla tamamlanıp yayımlandığında (`insert_staging`).

---

## 5. Güvenlik ve Endpoint Otoritesi

### 5.1 Endpoint Doğrulama ve Önizleme
- `preview_ai_endpoint`: Yalnızca sunum amaçlı (presentation-only) URL ayrıştırma ve sınıflandırma yapar; aktif endpoint yapılandırmasını veya registry state'ini değiştirmez.
- `configure_ai_endpoint`: URL'i bağımsız olarak yeniden doğrular, DNS çözümlemesi yapar, IP sabitleme (`resolve_to_addrs`) uygular, yetkilendirmeyi denetleyip aktif profili günceller ve `vector_store.clear_all()` ile önbelleği temizler.

### 5.2 URL & Network Sınırları
- Maksimum URL uzunluğu: `2048` karakter (ASCII).
- Userinfo (`user:pass@`), Query (`?`), Fragment (`#`) ve Root dışı path (`/api`) fail-closed olarak reddedilir.
- Şema kuralları: `Loopback` için HTTP/HTTPS; `PrivateNetwork` ve `RemotePublic` için HTTPS zorunludur.
- IP Sınıflandırması:
  - `Loopback`: 127.0.0.0/8, ::1, ::ffff:127.x.x.x.
  - `PrivateNetwork`: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, fc00::/7, ::ffff:10.x, ::ffff:192.168.x.
  - `PublicRoutable`: Genel internet routable IP'leri.
  - `Rejected`: 0.0.0.0, ::, 169.254.0.0/16, 100.64.0.0/10, 192.0.0.0/24, 192.0.2.0/24, 192.88.99.0/24, 198.18.0.0/15, 198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4, 240.0.0.0/4, 100::/64, 2001:2::/48, 2001:20::/28, 2001:db8::/32, fec0::/10.

### 5.3 HTTP İstemci Güvenliği ve Protokol Başlık Sınırları
- `Accept-Encoding: identity` zorunludur; `Content-Encoding` başlığı boş veya "identity" dışında ise `AI_ENDPOINT_DECOMPRESSION_REJECTED` döner (`gzip`, `br`, `deflate` reddedilir).
- 3xx Yönlendirmeler: 301, 302, 307, 308 tamamı `AI_ENDPOINT_REDIRECT_REJECTED` ile reddedilir.
- Proxy İzolasyonu: `no_proxy()` ile `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` çevresel değişkenleri yok sayılır.
- HTTP Zaman Aşımları ve Bağlantı Ayrımı:
  - Connect Timeout: 5s (`AI_ENDPOINT_CONNECT_TIMEOUT`). Reqwest client builder seviyesinde `.connect_timeout(5s)` ile yapılandırılmıştır. Gerçek zaman aşımı `e.is_connect() && e.is_timeout()` ile tespit edilir.
  - Connect Failed: Bağlantı reddi (`ECONNREFUSED`), host erişilemezliği veya TCP reset durumlarında `AI_ENDPOINT_CONNECT_FAILED` üretilir; asla sahte timeout olarak maskelenmez.
  - Header Timeout: 10s (`AI_ENDPOINT_HEADER_TIMEOUT`).
  - Read Idle Timeout: 30s (`AI_ENDPOINT_READ_TIMEOUT`).
  - Toplam İşlem Zaman Aşımları: RAG 120s / Özet 60s / İndeksleme 300s (`AI_ENDPOINT_TOTAL_TIMEOUT`).
- Yanıt Boyutu: Akışsız yanıtlar maks 5 MB (`AI_ENDPOINT_RESPONSE_TOO_LARGE`), akışlı toplam gövde maks 10 MB.
- **Sıfır Hata Gövdesi Saklama (Option A Zero Error Body Retention)**: HTTP hata durumlarında (non-2xx) yanıt gövdesi bellekte tutulmaz veya frontend'e aktarılmaz; yalnızca `AiError::EndpointHttpStatus(status_code)` döner.
- **Protokol-Özgü Yanıt Başlığı Denetimi**:
  - `HTTP/2`: `MAX_HTTP2_HEADER_LIST_BYTES = 16 * 1024` (16 KiB) ile reqwest builder seviyesinde kesin sınırlandırılmıştır (`.http2_max_header_list_size(MAX_HTTP2_HEADER_LIST_BYTES)`).
  - `HTTP/1.1`: reqwest/hyper dahili başlık ayrıştırma tamponu ve 10 saniyelik kesin başlık zaman aşımı (`header_timeout`) ile zaman-sınırlı (time-bound) korunur.

| Protokol | Başlık Bayt Sınırı | Zaman Sınırı | Güvenlik Sınıflandırması |
| :--- | :--- | :--- | :--- |
| **HTTP/1.1** | Hyper dahili ayrıştırma tamponu | 10s header timeout | Time-bound |
| **HTTP/2** | 16 KiB (`MAX_HTTP2_HEADER_LIST_BYTES`) | 10s header timeout | Byte-bound & Time-bound |

---

## 6. Girdi Doğrulama ve Kesin Sınırlar

| Parametre | Maksimum Sınır | Tipik Hata Kodu |
| :--- | :--- | :--- |
| Model Adı | 128 karakter (ASCII, `^[a-zA-Z0-9_.:/-]+$`) | `AI_INVALID_MODEL` |
| Chunk Size / Overlap | Chunk Size: 4096 char, Overlap: 2048 char (0 < overlap < size) | `AI_INVALID_CHUNK_POLICY` |
| Semantik Arama Sorgusu | 1000 karakter, Top-K: [1, 20] | `AI_REQUEST_INVALID` |
| Sohbet İstemi (Prompt) | 10,000 karakter ve 32,768 UTF-8 bayt | `AI_INVALID_PROMPT` |
| Sohbet Geçmişi | 50 mesaj; tekil mesaj maks 32,768 bayt; toplam maks 102,400 bayt | `AI_INVALID_HISTORY` |
| Özetleme Seçimi | 50,000 karakter; Target Length: `short`, `detailed`, `bullet` | `AI_INVALID_PROMPT` / `AI_REQUEST_INVALID` |
| Belge İndeksleme | 1,000 sayfa, 5,000,000 karakter, 20,000,000 bayt, tek chunk maks 16,384 bayt, maks 10,000 chunk | `AI_INDEX_LIMIT_EXCEEDED` |
| İstek / Deneme ID | `req_<uuid_v4>` / `attempt_<uuid_v4>` formatı (küçük harf, RFC 4122 v4 UUID) | `AI_REQUEST_INVALID` |

---

## 7. VectorStore ve Bellek Yaşam Döngüsü

- **Vektör Doğrulama**: NaN, +Infinity, -Infinity reddedilir (`AI_EMBEDDING_INVALID`); maks vektör boyutu 4096; kanonik boyut uyumsuzluğunda `AI_EMBEDDING_DIMENSION_MISMATCH`.
- **Kapasite ve Sınırlar**: Oturum başı maks 50,000 vektör ve 100 MB; küresel maks 500 MB. Aşımda `AI_VECTOR_STORE_LIMIT_EXCEEDED`.
- **Parça ve Model Eşleme**: `matches_expected_configuration` ile `(generation, endpoint_fingerprint, embed_model, chunk_size, chunk_overlap)` 5'li tuple olarak doğrulanır.
- **Atomik İndeks Geri Alma (I6)**: Çok parçalı indeks derlemesinde herhangi bir parça başarısız olduğunda hiçbir parça yayımlanmaz; bellek 0 kalır veya eski indeks korunur.
- **Yeniden Derleme İzolasyonu (I7)**: Geçersiz vektör veya boyut uyumsuzluğu içeren yeniden derleme denemeleri mevcut geçerli indeksi bozmaz.

---

## 8. Frontend Veri Akışı Şeffaflığı ve Yetkilendirme (Data-Flow Disclosures & Consent Guards)

Kullanıcı arayüzünde aktif operasyon türüne ve seçili profile göre şeffaf veri akışı bildirimleri gösterilir:
1. **İndeksleme Modu**: `"{modeLabel}: Belgeden çıkarılan metin parçaları ve embedding model adı ({embedModel}) yapılandırılan endpoint'e iletilir."`
2. **Sohbet (RAG) Modu**: `"{modeLabel}: Kullanıcı sorusu, getirilen belge bağlam parçaları, sınırlandırılmış sohbet geçmişi ve LLM model adı ({llmModel}) yapılandırılan endpoint'e iletilir; semantik erişim sırasında embedding modeli ({embedModel}) ile iletişim kurulabilir."`
3. **Metin Seçimi Özeti**: `"{modeLabel}: Seçili metin, özetleme/açıklama talimatı ve LLM model adı ({llmModel}) yapılandırılan endpoint'e iletilir."`

### 8.1 4 Operasyonluk Fail-Closed Frontend Onay Denetimi (Consent Guard)
Frontend `aiController`, Tauri backend'e IPC çağrısı yapmadan önce 4 AI operasyonunun tamamında (`buildSemanticIndex`, `semanticSearch`, `startQueryDocumentRag`, `startSummary`) `_checkAuthorization()` denetimi uygular:
- **Loopback**: Onay gerekmeksizin tüm 4 operasyon doğrudan çalışır.
- **RemotePublic**: `remoteConsent` eksikse `AI_ENDPOINT_REMOTE_CONSENT_REQUIRED` hatası fırlatılır.
- **PrivateNetwork**: `remoteConsent` veya `privateConsent` eksikse `AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED` hatası fırlatılır.
- **Operasyonel İzolasyon**: Onay reddinde `_requests` kaydı açılmaz (0), `activeStreamingCount` artırılmaz (0), `isIndexing` false kalır ve backend IPC çağrısı yapılmaz (`callBackend` count = 0).
- **Senkron Handle Sözleşmesi**: `startQueryDocumentRag` ve `startSummary` senkron olarak `{ requestId, cancel, done, subscribe }` döndürür; yetkisiz çağrılarda `done` promise typed consent hatası ile anında reject edilir.

---

## 9. Akış Ayrıştırıcı ve Terminal Teslimat

- **NdjsonStreamParser**: Satır sonu (`\n`) ayrıştırması önce tüm eksiksiz satırları tüketir; kalan tamamlanmamış buffer maks 64 KB ile sınırlıdır (`AI_ENDPOINT_STREAM_FRAME_TOO_LARGE`). Toplam akış maks 10 MB, maks 5,000 event (`AI_STREAM_TOTAL_LIMIT_EXCEEDED`).
- **Fail-Closed EOF Kontrolü (`check_eof`)**: Akış tamamlanma sinyali (`done: true`) almadan sonlanırsa veya buffer'da artık bayt kalırsa `AI_ENDPOINT_STREAM_PROTOCOL_ERROR` döner.
- **Terminal Teslimat Durum Makinesi**:
  - `Pending -> Delivering -> Delivered` (veya `FailedDelivery`).
  - Terminal event (`completed`, `failed`, `cancelled`) yalnızca tek bir kez iletilir; mükerrer çağrılar idempotent korunur.

---

## 10. Frontend Yaşam Döngüsü ve Svelte 5 Runes

- Svelte 5 Runes (`$state`, `$derived`, `$props`, `$effect`) mimarisi.
- `aiState.activeProfile` başlangıçta `null` olarak başlatılır; `isHydrated` durumu `Boolean(activeProfile?.endpoint_fingerprint)` ile türetilir.
- `isIndexReadyFor(sessionId, generation)`: `activeProfile` veya fingerprint eksikse fail-closed olarak `false` döner; oturum ID, jenerasyon, embedModel, chunkSize, chunkOverlap ve endpointFingerprint tam eşleşmesini kontrol eder.
- **Depolama Göçü (`_initStorageMigration`)**: Yalnızca `schemaVersion === 1` nesnelerini kabul eder; URL uzunluğu (<= 2048) ve model adlarını (`/^[a-zA-Z0-9_.:/-]+$/`) doğrular; onay (consent) alanları asla localStorage'a kaydedilmez veya yüklenmez.
- `aiController.dispose()`: `applicationRuntime.js` unmount akışında aktif istekleri iptal eder, stream dinleyicisini kaldırır ve bellek durumunu sıfırlar.
