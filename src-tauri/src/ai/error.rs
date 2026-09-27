use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    EndpointInvalid(String),
    EndpointSchemeNotAllowed(String),
    EndpointHostNotAllowed(String),
    EndpointPathNotAllowed(String),
    EndpointCredentialsNotAllowed(String),
    EndpointRemoteConsentRequired(String),
    EndpointPrivateConsentRequired(String),
    EndpointDnsResolutionFailed(String),
    EndpointDnsTimeout(String),
    EndpointDnsPolicyViolation(String),
    EndpointPinningUnavailable(String),
    EndpointRedirectRejected(String),
    EndpointConnectFailed(String),
    EndpointConnectTimeout(String),
    EndpointHeaderTimeout(String),
    EndpointReadTimeout(String),
    EndpointTotalTimeout(String),
    EndpointHttpStatus(u16),
    EndpointContentTypeInvalid(String),
    EndpointDecompressionRejected(String),
    EndpointResponseTooLarge(String),
    EndpointStreamFrameTooLarge(String),
    EndpointStreamProtocolError(String),
    RequestInvalid(String),
    RequestLimitExceeded(String),
    RequestDuplicate(String),
    RequestNotFound(String),
    RequestAlreadyTerminal(String),
    RequestOwnerMismatch(String),
    RequestNotCancellable(String),
    CancellationChannelClosed(String),
    RequestCancelled(String),
    InvalidModel(String),
    InvalidChunkPolicy(String),
    InvalidHistory(String),
    InvalidPrompt(String),
    IndexLimitExceeded(String),
    IndexIncomplete(String),
    IndexConfigMismatch(String),
    IndexNotFound(String),
    IndexOutdated(String),
    StreamTotalLimitExceeded(String),
    StreamUnexpectedEof(String),
    StreamSequenceError(String),
    EmbeddingInvalid(String),
    EmbeddingDimensionMismatch(String),
    IndexPartialFailure(String),
    VectorStoreLimitExceeded,
}

// Aliases for backwards compatibility within codebase
#[allow(non_upper_case_globals)]
impl AiError {
    pub fn stream_frame_too_large(msg: String) -> Self {
        AiError::EndpointStreamFrameTooLarge(msg)
    }

    pub fn stream_protocol_error(msg: String) -> Self {
        AiError::EndpointStreamProtocolError(msg)
    }
}

impl AiError {
    pub fn code(&self) -> &'static str {
        match self {
            AiError::EndpointInvalid(_) => "AI_ENDPOINT_INVALID",
            AiError::EndpointSchemeNotAllowed(_) => "AI_ENDPOINT_SCHEME_NOT_ALLOWED",
            AiError::EndpointHostNotAllowed(_) => "AI_ENDPOINT_HOST_NOT_ALLOWED",
            AiError::EndpointPathNotAllowed(_) => "AI_ENDPOINT_PATH_NOT_ALLOWED",
            AiError::EndpointCredentialsNotAllowed(_) => "AI_ENDPOINT_CREDENTIALS_NOT_ALLOWED",
            AiError::EndpointRemoteConsentRequired(_) => "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
            AiError::EndpointPrivateConsentRequired(_) => {
                "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED"
            }
            AiError::EndpointDnsResolutionFailed(_) => "AI_ENDPOINT_DNS_RESOLUTION_FAILED",
            AiError::EndpointDnsTimeout(_) => "AI_ENDPOINT_DNS_TIMEOUT",
            AiError::EndpointDnsPolicyViolation(_) => "AI_ENDPOINT_DNS_POLICY_VIOLATION",
            AiError::EndpointPinningUnavailable(_) => "AI_ENDPOINT_PINNING_UNAVAILABLE",
            AiError::EndpointRedirectRejected(_) => "AI_ENDPOINT_REDIRECT_REJECTED",
            AiError::EndpointConnectFailed(_) => "AI_ENDPOINT_CONNECT_FAILED",
            AiError::EndpointConnectTimeout(_) => "AI_ENDPOINT_CONNECT_TIMEOUT",
            AiError::EndpointHeaderTimeout(_) => "AI_ENDPOINT_HEADER_TIMEOUT",
            AiError::EndpointReadTimeout(_) => "AI_ENDPOINT_READ_TIMEOUT",
            AiError::EndpointTotalTimeout(_) => "AI_ENDPOINT_TOTAL_TIMEOUT",
            AiError::EndpointHttpStatus(_) => "AI_ENDPOINT_HTTP_STATUS",
            AiError::EndpointContentTypeInvalid(_) => "AI_ENDPOINT_CONTENT_TYPE_INVALID",
            AiError::EndpointDecompressionRejected(_) => "AI_ENDPOINT_DECOMPRESSION_REJECTED",
            AiError::EndpointResponseTooLarge(_) => "AI_ENDPOINT_RESPONSE_TOO_LARGE",
            AiError::EndpointStreamFrameTooLarge(_) => "AI_ENDPOINT_STREAM_FRAME_TOO_LARGE",
            AiError::EndpointStreamProtocolError(_) => "AI_ENDPOINT_STREAM_PROTOCOL_ERROR",
            AiError::RequestInvalid(_) => "AI_REQUEST_INVALID",
            AiError::RequestLimitExceeded(_) => "AI_REQUEST_LIMIT_EXCEEDED",
            AiError::RequestDuplicate(_) => "AI_REQUEST_DUPLICATE",
            AiError::RequestNotFound(_) => "AI_REQUEST_NOT_FOUND",
            AiError::RequestAlreadyTerminal(_) => "AI_REQUEST_ALREADY_TERMINAL",
            AiError::RequestOwnerMismatch(_) => "AI_REQUEST_OWNER_MISMATCH",
            AiError::RequestNotCancellable(_) => "AI_REQUEST_NOT_CANCELLABLE",
            AiError::CancellationChannelClosed(_) => "AI_CANCELLATION_CHANNEL_CLOSED",
            AiError::RequestCancelled(_) => "AI_REQUEST_CANCELLED",
            AiError::InvalidModel(_) => "AI_INVALID_MODEL",
            AiError::InvalidChunkPolicy(_) => "AI_INVALID_CHUNK_POLICY",
            AiError::InvalidHistory(_) => "AI_INVALID_HISTORY",
            AiError::InvalidPrompt(_) => "AI_INVALID_PROMPT",
            AiError::IndexLimitExceeded(_) => "AI_INDEX_LIMIT_EXCEEDED",
            AiError::IndexIncomplete(_) => "AI_INDEX_INCOMPLETE",
            AiError::IndexConfigMismatch(_) => "AI_INDEX_CONFIG_MISMATCH",
            AiError::IndexNotFound(_) => "AI_INDEX_NOT_FOUND",
            AiError::IndexOutdated(_) => "AI_INDEX_OUTDATED",
            AiError::StreamTotalLimitExceeded(_) => "AI_STREAM_TOTAL_LIMIT_EXCEEDED",
            AiError::StreamUnexpectedEof(_) => "AI_STREAM_UNEXPECTED_EOF",
            AiError::StreamSequenceError(_) => "AI_STREAM_SEQUENCE_ERROR",
            AiError::EmbeddingInvalid(_) => "AI_EMBEDDING_INVALID",
            AiError::EmbeddingDimensionMismatch(_) => "AI_EMBEDDING_DIMENSION_MISMATCH",
            AiError::IndexPartialFailure(_) => "AI_INDEX_PARTIAL_FAILURE",
            AiError::VectorStoreLimitExceeded => "AI_VECTOR_STORE_LIMIT_EXCEEDED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            AiError::EndpointInvalid(msg) => format!("Endpoint URL biçimi geçersiz: {}", msg),
            AiError::EndpointSchemeNotAllowed(msg) => format!(
                "Endpoint şeması (HTTP/HTTPS) bu mod için izin verilmiyor: {}",
                msg
            ),
            AiError::EndpointHostNotAllowed(msg) => {
                format!("Endpoint adresi bu mod için izin verilmiyor: {}", msg)
            }
            AiError::EndpointPathNotAllowed(msg) => {
                format!("Endpoint URL base path içeremez: {}", msg)
            }
            AiError::EndpointCredentialsNotAllowed(_) => {
                "Endpoint URL kullanıcı kimlik bilgisi (userinfo) içeremez.".to_string()
            }
            AiError::EndpointRemoteConsentRequired(origin) => format!(
                "Uzak AI endpoint kullanımı için onay gereklidir: {}",
                origin
            ),
            AiError::EndpointPrivateConsentRequired(origin) => format!(
                "Özel ağ AI endpoint kullanımı için onay gereklidir: {}",
                origin
            ),
            AiError::EndpointDnsResolutionFailed(host) => {
                format!("Endpoint DNS alan adı çözümlenemedi: {}", host)
            }
            AiError::EndpointDnsTimeout(host) => format!(
                "Endpoint DNS alan adı çözümlemesi zaman aşımına uğradı (3s): {}",
                host
            ),
            AiError::EndpointDnsPolicyViolation(msg) => {
                format!("DNS sonucu izin verilmeyen IP adresine çözüldü: {}", msg)
            }
            AiError::EndpointPinningUnavailable(host) => format!(
                "Hostname IP sabitleme (pinning) gerçekleştirilemedi: {}",
                host
            ),
            AiError::EndpointRedirectRejected(_) => {
                "Endpoint yönlendirme isteği reddedildi.".to_string()
            }
            AiError::EndpointConnectFailed(msg) => {
                format!("Endpoint sunucusuna bağlantı kurulamadı: {}", msg)
            }
            AiError::EndpointHttpStatus(status) => {
                format!("Endpoint HTTP hatası döndürdü ({})", status)
            }
            AiError::EndpointContentTypeInvalid(ct) => {
                format!("Beklenmeyen Content-Type yanıtı alındı: {}", ct)
            }
            AiError::EndpointDecompressionRejected(enc) => {
                format!("Sıkıştırılmış yanıt (Content-Encoding) reddedildi: {}", enc)
            }
            AiError::EndpointConnectTimeout(stage) => format!(
                "Endpoint sunucusuna bağlanma zaman aşımına uğradı (5s): {}",
                stage
            ),
            AiError::EndpointHeaderTimeout(stage) => format!(
                "Endpoint yanıt başlığı bekleme zaman aşımına uğradı (10s): {}",
                stage
            ),
            AiError::EndpointReadTimeout(stage) => format!(
                "Endpoint veri okuma yanıt vermedi (30s read-idle): {}",
                stage
            ),
            AiError::EndpointTotalTimeout(stage) => {
                format!("Yapay zeka işlemi toplam zaman aşımına uğradı: {}", stage)
            }
            AiError::EndpointResponseTooLarge(sz) => {
                format!("Endpoint gövde boyutu 5 MB sınırını aştı: {}", sz)
            }
            AiError::EndpointStreamFrameTooLarge(sz) => {
                format!("Akış paket satırı maksimum 64 KB sınırını aştı: {}", sz)
            }
            AiError::EndpointStreamProtocolError(err) => {
                format!("Akış paket biçimi veya JSON yapısı geçersiz: {}", err)
            }
            AiError::RequestInvalid(msg) => {
                format!("Yapay zeka isteği geçersiz: {}", msg)
            }
            AiError::RequestLimitExceeded(msg) => format!(
                "Eşzamanlı maksimum yapay zeka isteği sınırına ulaşıldı: {}",
                msg
            ),
            AiError::RequestDuplicate(id) => {
                format!("Bu istek kimliği (requestId) zaten mevcut: {}", id)
            }
            AiError::RequestNotFound(id) => {
                format!("İptal edilecek aktif istek bulunamadı: {}", id)
            }
            AiError::RequestAlreadyTerminal(id) => {
                format!("İstek zaten tamamlanmış veya iptal edilmiş: {}", id)
            }
            AiError::RequestOwnerMismatch(_) => {
                "İstek sahipliği, oturum veya attempt ID eşleşmedi.".to_string()
            }
            AiError::RequestNotCancellable(id) => format!("Bu istek iptal edilemez: {}", id),
            AiError::CancellationChannelClosed(msg) => format!("İptal kanalı kapatıldı: {}", msg),
            AiError::RequestCancelled(_) => {
                "Yapay zeka isteği kullanıcı tarafından iptal edildi.".to_string()
            }
            AiError::InvalidModel(msg) => format!("Model adı geçersiz: {}", msg),
            AiError::InvalidChunkPolicy(msg) => format!("Chunk yapılandırması geçersiz: {}", msg),
            AiError::InvalidHistory(msg) => format!("Sohbet geçmişi geçersiz: {}", msg),
            AiError::InvalidPrompt(msg) => format!("İstem metni geçersiz: {}", msg),
            AiError::IndexLimitExceeded(msg) => format!("İndeks sınırları aşıldı: {}", msg),
            AiError::IndexIncomplete(msg) => format!("İndeks tamamlanamadı: {}", msg),
            AiError::IndexConfigMismatch(msg) => {
                format!("İndeks yapılandırma uyumsuzluğu: {}", msg)
            }
            AiError::IndexNotFound(msg) => format!("Belge indeksi bulunamadı: {}", msg),
            AiError::IndexOutdated(msg) => {
                format!("Belge indeksi güncel değil: {}", msg)
            }
            AiError::StreamTotalLimitExceeded(sz) => {
                format!("Akış toplam boyutu 10 MB sınırını aştı: {}", sz)
            }
            AiError::StreamUnexpectedEof(_) => "Akış tamamlanmadan bağlantı kesildi.".to_string(),
            AiError::StreamSequenceError(seq) => format!("Akış paket sırası bozuldu: {}", seq),
            AiError::EmbeddingInvalid(idx) => format!(
                "Üretilen embedding vektöründe NaN veya Infinity var: {}",
                idx
            ),
            AiError::EmbeddingDimensionMismatch(msg) => {
                format!("Embedding vektör boyutu önbellekle eşleşmiyor: {}", msg)
            }
            AiError::IndexPartialFailure(msg) => format!(
                "İndeks oluşturma sırasında bir chunk başarısız oldu: {}",
                msg
            ),
            AiError::VectorStoreLimitExceeded => "Vector Store bellek sınırı aşıldı.".to_string(),
        }
    }

    pub fn to_wire(&self) -> AiErrorWire {
        AiErrorWire {
            code: self.code().to_string(),
            message: self.message(),
        }
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code(), self.message())
    }
}

impl std::error::Error for AiError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiErrorWire {
    pub code: String,
    pub message: String,
}

impl From<AiError> for AiErrorWire {
    fn from(err: AiError) -> Self {
        err.to_wire()
    }
}

impl From<AiError> for String {
    fn from(err: AiError) -> Self {
        err.to_string()
    }
}
