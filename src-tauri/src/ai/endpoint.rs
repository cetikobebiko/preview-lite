use crate::ai::error::AiError;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

pub const MAX_ENDPOINT_URL_LENGTH: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EndpointMode {
    Loopback,
    RemotePublic,
    PrivateNetwork,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiEndpointProfile {
    pub policy_version: u32,
    pub mode: EndpointMode,
    pub scheme: String,
    pub canonical_host: String,
    pub effective_port: u16,
    pub canonical_origin: String,
    pub endpoint_fingerprint: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiEndpointConsent {
    pub endpoint_fingerprint: String,
    pub remote_approved: bool,
    pub private_network_approved: bool,
    pub policy_version: u32,
    pub updated_at_ms: u64,
}

pub struct AiEndpointRuntimeContext {
    pub profile: Arc<AiEndpointProfile>,
    pub consent: Arc<AiEndpointConsent>,
    pub client: Arc<reqwest::Client>,
    pub validated_addresses: Vec<SocketAddr>,
    pub resolved_at_ms: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum IpCategory {
    Loopback,
    Private,
    PublicRoutable,
    Rejected,
}

pub fn classify_ip(ip: &IpAddr) -> IpCategory {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // IPv4 Loopback: 127.0.0.0/8
            if octets[0] == 127 {
                return IpCategory::Loopback;
            }
            // Unspecified: 0.0.0.0/8
            if octets[0] == 0 {
                return IpCategory::Rejected;
            }
            // RFC1918 Private IPv4:
            // 10.0.0.0/8
            if octets[0] == 10 {
                return IpCategory::Private;
            }
            // 172.16.0.0/12
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return IpCategory::Private;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return IpCategory::Private;
            }
            // Link-local: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 {
                return IpCategory::Rejected;
            }
            // CGNAT: 100.64.0.0/10 (100.64.0.0 - 100.127.255.255)
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return IpCategory::Rejected;
            }
            // IETF Protocol Assignments: 192.0.0.0/24 (RFC 6890)
            if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
                return IpCategory::Rejected;
            }
            // Documentation: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
            if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
                || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
                || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
            {
                return IpCategory::Rejected;
            }
            // 6to4 Relay Anycast: 192.88.99.0/24 (RFC 7526)
            if octets[0] == 192 && octets[1] == 88 && octets[2] == 99 {
                return IpCategory::Rejected;
            }
            // Benchmarking: 198.18.0.0/15
            if octets[0] == 198 && (18..=19).contains(&octets[1]) {
                return IpCategory::Rejected;
            }
            // Multicast / Broadcast / Reserved: 224.0.0.0/4, 255.255.255.255
            if octets[0] >= 224 {
                return IpCategory::Rejected;
            }

            IpCategory::PublicRoutable
        }
        IpAddr::V6(ipv6) => {
            // IPv4-mapped IPv6 ::ffff:a.b.c.d normalized to IPv4 rules
            if let Some(ipv4) = ipv6.to_ipv4_mapped() {
                return classify_ip(&IpAddr::V4(ipv4));
            }

            // IPv6 Loopback ::1
            if ipv6.is_loopback() {
                return IpCategory::Loopback;
            }
            // Unspecified ::
            if ipv6.is_unspecified() {
                return IpCategory::Rejected;
            }

            let segments = ipv6.segments();
            // Discard-only prefix 100::/64 (RFC 6666)
            if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
                return IpCategory::Rejected;
            }
            // IPv6 Benchmarking 2001:2::/48 (RFC 5180)
            if segments[0] == 0x2001 && segments[1] == 0x0002 && segments[2] == 0 {
                return IpCategory::Rejected;
            }
            // IPv6 ORCHIDv2 2001:20::/28 (RFC 7343)
            if segments[0] == 0x2001 && (segments[1] & 0xfff0) == 0x0020 {
                return IpCategory::Rejected;
            }
            // IPv6 Documentation 2001:db8::/32
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return IpCategory::Rejected;
            }
            // IPv6 ULA fc00::/7 (fc00 - fdff)
            if (segments[0] & 0xfe00) == 0xfc00 {
                return IpCategory::Private;
            }
            // IPv6 Link-Local fe80::/10
            if (segments[0] & 0xffc0) == 0xfe80 {
                return IpCategory::Rejected;
            }
            // Deprecated IPv6 Site-Local fec0::/10 (RFC 3879)
            if (segments[0] & 0xffc0) == 0xfec0 {
                return IpCategory::Rejected;
            }
            // IPv6 Multicast ff00::/8
            if (segments[0] & 0xff00) == 0xff00 {
                return IpCategory::Rejected;
            }

            IpCategory::PublicRoutable
        }
    }
}

pub fn compute_endpoint_fingerprint(
    policy_version: u32,
    mode: &EndpointMode,
    scheme: &str,
    host: &str,
    port: u16,
) -> String {
    let mode_str = match mode {
        EndpointMode::Loopback => "loopback",
        EndpointMode::RemotePublic => "remote_public",
        EndpointMode::PrivateNetwork => "private_network",
    };
    let scheme_norm = scheme.to_lowercase();
    let host_norm = canonicalize_hostname_string(host);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&policy_version.to_be_bytes()); // 4 bytes

    bytes.extend_from_slice(&(mode_str.len() as u32).to_be_bytes()); // 4 bytes
    bytes.extend_from_slice(mode_str.as_bytes());

    bytes.extend_from_slice(&(scheme_norm.len() as u32).to_be_bytes()); // 4 bytes
    bytes.extend_from_slice(scheme_norm.as_bytes());

    bytes.extend_from_slice(&(host_norm.len() as u32).to_be_bytes()); // 4 bytes
    bytes.extend_from_slice(host_norm.as_bytes());

    bytes.extend_from_slice(&port.to_be_bytes()); // 2 bytes

    format!("{:x}", Sha256::digest(&bytes))
}

pub fn canonicalize_hostname_string(host: &str) -> String {
    let mut h = host.trim().to_lowercase();
    if h.ends_with('.') {
        h.pop();
    }
    h
}

pub fn is_authorized(
    mode: &EndpointMode,
    consent: Option<&AiEndpointConsent>,
    active_fingerprint: &str,
) -> bool {
    let consent = match consent {
        Some(c) if c.endpoint_fingerprint == active_fingerprint && c.policy_version == 1 => c,
        _ => return matches!(mode, EndpointMode::Loopback),
    };

    match mode {
        EndpointMode::Loopback => true,
        EndpointMode::RemotePublic => consent.remote_approved,
        EndpointMode::PrivateNetwork => consent.remote_approved && consent.private_network_approved,
    }
}

pub fn parse_and_validate_url(raw_url: &str) -> Result<(String, String, u16), AiError> {
    let trimmed = raw_url.trim();
    if trimmed.len() > MAX_ENDPOINT_URL_LENGTH {
        return Err(AiError::EndpointInvalid(format!(
            "URL length exceeds {} characters",
            MAX_ENDPOINT_URL_LENGTH
        )));
    }

    let url = Url::parse(trimmed).map_err(|e| AiError::EndpointInvalid(e.to_string()))?;

    let scheme = url.scheme().to_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(AiError::EndpointSchemeNotAllowed(scheme));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::EndpointCredentialsNotAllowed(
            "Userinfo in URL is forbidden".to_string(),
        ));
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(AiError::EndpointPathNotAllowed(
            "Query or fragment in URL is forbidden".to_string(),
        ));
    }

    let path = url.path();
    if path != "/" && !path.is_empty() {
        return Err(AiError::EndpointPathNotAllowed(format!(
            "Base path '{}' is forbidden",
            path
        )));
    }

    let host_str = url
        .host_str()
        .ok_or_else(|| AiError::EndpointInvalid("Missing host".to_string()))?;

    let canonical_host = canonicalize_hostname_string(host_str);
    if canonical_host.is_empty() {
        return Err(AiError::EndpointInvalid("Empty host".to_string()));
    }

    let effective_port = url
        .port_or_known_default()
        .ok_or_else(|| AiError::EndpointInvalid("Unknown default port for scheme".to_string()))?;

    Ok((scheme, canonical_host, effective_port))
}

pub fn validate_and_classify_addresses(
    scheme: &str,
    canonical_host: &str,
    addrs: &[SocketAddr],
) -> Result<EndpointMode, AiError> {
    if addrs.is_empty() {
        return Err(AiError::EndpointDnsResolutionFailed(format!(
            "No IP addresses resolved for {}",
            canonical_host
        )));
    }

    let mut categories = Vec::new();
    for sa in addrs {
        let cat = classify_ip(&sa.ip());
        if cat == IpCategory::Rejected {
            return Err(AiError::EndpointDnsPolicyViolation(format!(
                "Resolved IP {} belongs to a forbidden IP category",
                sa.ip()
            )));
        }
        categories.push(cat);
    }

    let first_cat = categories[0];
    for cat in &categories {
        if *cat != first_cat {
            return Err(AiError::EndpointDnsPolicyViolation(format!(
                "Mixed IP categories resolved for {}",
                canonical_host
            )));
        }
    }

    let mode = match first_cat {
        IpCategory::Loopback => EndpointMode::Loopback,
        IpCategory::Private => {
            if scheme != "https" {
                return Err(AiError::EndpointSchemeNotAllowed(
                    "Private network endpoints require HTTPS".to_string(),
                ));
            }
            EndpointMode::PrivateNetwork
        }
        IpCategory::PublicRoutable => {
            if scheme != "https" {
                return Err(AiError::EndpointSchemeNotAllowed(
                    "Remote public endpoints require HTTPS".to_string(),
                ));
            }
            EndpointMode::RemotePublic
        }
        IpCategory::Rejected => unreachable!(),
    };

    Ok(mode)
}

pub async fn resolve_and_classify_endpoint(
    scheme: &str,
    canonical_host: &str,
    port: u16,
) -> Result<(EndpointMode, Vec<SocketAddr>), AiError> {
    let target = format!("{}:{}", canonical_host, port);

    let addrs: Vec<SocketAddr> =
        match timeout(Duration::from_secs(3), tokio::net::lookup_host(&target)).await {
            Ok(Ok(iter)) => iter.collect(),
            Ok(Err(e)) => {
                return Err(AiError::EndpointDnsResolutionFailed(format!(
                    "{}: {}",
                    canonical_host, e
                )))
            }
            Err(_) => return Err(AiError::EndpointDnsTimeout(canonical_host.to_string())),
        };

    let mode = validate_and_classify_addresses(scheme, canonical_host, &addrs)?;
    Ok((mode, addrs))
}

pub const MAX_HTTP2_HEADER_LIST_BYTES: u32 = 16 * 1024;
pub(crate) const SECURE_CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn build_secure_reqwest_client(
    canonical_host: &str,
    addrs: &[SocketAddr],
) -> Result<reqwest::Client, AiError> {
    build_secure_reqwest_client_with_connect_timeout(
        canonical_host,
        addrs,
        SECURE_CLIENT_CONNECT_TIMEOUT,
    )
}

pub(crate) fn build_secure_reqwest_client_with_connect_timeout(
    canonical_host: &str,
    addrs: &[SocketAddr],
    connect_timeout: Duration,
) -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .resolve_to_addrs(canonical_host, addrs)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .http2_max_header_list_size(MAX_HTTP2_HEADER_LIST_BYTES)
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| AiError::EndpointInvalid(e.to_string()))
}

pub fn create_default_loopback_profile() -> AiEndpointProfile {
    let scheme = "http".to_string();
    let host = "127.0.0.1".to_string();
    let port = 11434;
    let mode = EndpointMode::Loopback;
    let fingerprint = compute_endpoint_fingerprint(1, &mode, &scheme, &host, port);
    let canonical_origin = format!("{}://{}:{}", scheme, host, port);

    AiEndpointProfile {
        policy_version: 1,
        mode,
        scheme,
        canonical_host: host,
        effective_port: port,
        canonical_origin,
        endpoint_fingerprint: fingerprint,
        created_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}

pub struct AiEndpointStore {
    runtime_context: parking_lot::RwLock<Option<Arc<AiEndpointRuntimeContext>>>,
    persisted_consent: parking_lot::RwLock<Option<Arc<AiEndpointConsent>>>,
}

impl AiEndpointStore {
    pub fn new() -> Self {
        Self {
            runtime_context: parking_lot::RwLock::new(None),
            persisted_consent: parking_lot::RwLock::new(None),
        }
    }

    pub fn get_active_context(&self) -> Result<Arc<AiEndpointRuntimeContext>, AiError> {
        let guard = self.runtime_context.read();
        if let Some(ctx) = guard.as_ref() {
            Ok(ctx.clone())
        } else {
            // Default loopback fallback
            drop(guard);
            self.ensure_default_profile()
        }
    }

    pub fn ensure_default_profile(&self) -> Result<Arc<AiEndpointRuntimeContext>, AiError> {
        let mut guard = self.runtime_context.write();
        if let Some(ctx) = guard.as_ref() {
            return Ok(ctx.clone());
        }

        let default_profile = Arc::new(create_default_loopback_profile());
        let default_consent = Arc::new(AiEndpointConsent {
            endpoint_fingerprint: default_profile.endpoint_fingerprint.clone(),
            remote_approved: false,
            private_network_approved: false,
            policy_version: 1,
            updated_at_ms: default_profile.created_at_ms,
        });

        // Loopback dummy validated address
        let addrs = vec!["127.0.0.1:11434".parse().unwrap()];

        let client = Arc::new(build_secure_reqwest_client(
            &default_profile.canonical_host,
            &addrs,
        )?);

        let ctx = Arc::new(AiEndpointRuntimeContext {
            profile: default_profile,
            consent: default_consent,
            client,
            validated_addresses: addrs,
            resolved_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });

        *guard = Some(ctx.clone());
        Ok(ctx)
    }

    pub fn set_runtime_context(&self, ctx: Arc<AiEndpointRuntimeContext>) {
        let mut guard = self.runtime_context.write();
        *guard = Some(ctx);
    }

    #[allow(clippy::result_large_err)]
    pub fn with_matching_fingerprint<
        R,
        F: FnOnce() -> Result<R, crate::document_session::DocumentSessionError>,
    >(
        &self,
        expected_fingerprint: &str,
        operation: F,
    ) -> Result<R, crate::document_session::DocumentSessionError> {
        let guard = self.runtime_context.read();
        let current_fingerprint = guard
            .as_ref()
            .map(|ctx| ctx.profile.endpoint_fingerprint.clone())
            .unwrap_or_else(|| create_default_loopback_profile().endpoint_fingerprint);
        if current_fingerprint != expected_fingerprint {
            return Err(crate::document_session::DocumentSessionError::new(
                "AI_INDEX_OUTDATED",
                format!(
                    "Endpoint fingerprint mismatch: expected {}, current {}",
                    expected_fingerprint, current_fingerprint
                ),
            ));
        }
        operation()
    }

    pub fn clear_config(&self) -> Arc<AiEndpointRuntimeContext> {
        {
            let mut consent_guard = self.persisted_consent.write();
            *consent_guard = None;
        }
        {
            let mut ctx_guard = self.runtime_context.write();
            *ctx_guard = None;
        }
        self.ensure_default_profile().unwrap()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiEndpointStatusDto {
    pub profile: AiEndpointProfile,
    pub consent: AiEndpointConsent,
    pub is_authorized: bool,
}

impl Default for AiEndpointStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    #[test]
    fn test_ip_classification() {
        assert_eq!(
            classify_ip(&"127.0.0.1".parse().unwrap()),
            IpCategory::Loopback
        );
        assert_eq!(
            classify_ip(&"127.0.1.5".parse().unwrap()),
            IpCategory::Loopback
        );
        assert_eq!(classify_ip(&"::1".parse().unwrap()), IpCategory::Loopback);
        assert_eq!(
            classify_ip(&"::ffff:127.0.0.1".parse().unwrap()),
            IpCategory::Loopback
        );

        assert_eq!(
            classify_ip(&"10.0.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"172.16.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"192.168.1.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"::ffff:10.0.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"fd00::1".parse().unwrap()),
            IpCategory::Private
        );

        assert_eq!(
            classify_ip(&"8.8.8.8".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"1.1.1.1".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"::ffff:8.8.8.8".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"2606:4700:4700::1111".parse().unwrap()),
            IpCategory::PublicRoutable
        );

        assert_eq!(
            classify_ip(&"169.254.1.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"::ffff:169.254.1.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"224.0.0.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"0.0.0.0".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"::ffff:0.0.0.0".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"100.64.0.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"192.0.2.1".parse().unwrap()),
            IpCategory::Rejected
        );
    }

    #[test]
    fn test_url_canonicalization_and_validation() {
        let (scheme, host, port) = parse_and_validate_url("http://127.0.0.1:11434").unwrap();
        assert_eq!(scheme, "http");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 11434);

        let (scheme2, host2, port2) = parse_and_validate_url("https://example.com/").unwrap();
        assert_eq!(scheme2, "https");
        assert_eq!(host2, "example.com");
        assert_eq!(port2, 443);

        // Path rejection
        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434/api"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));
        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434//api"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));

        // Userinfo rejection
        assert!(matches!(
            parse_and_validate_url("http://user:pass@127.0.0.1:11434"),
            Err(AiError::EndpointCredentialsNotAllowed(_))
        ));

        // Query/Fragment rejection
        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434?foo=bar"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));

        // Length limit rejection
        let long_url = format!("http://127.0.0.1:11434/{}", "a".repeat(2048));
        assert!(matches!(
            parse_and_validate_url(&long_url),
            Err(AiError::EndpointInvalid(_))
        ));
    }

    #[test]
    fn test_consent_predicate_truth_table() {
        let fp = "test_fp";
        let consent_none: Option<&AiEndpointConsent> = None;
        assert!(is_authorized(&EndpointMode::Loopback, consent_none, fp));
        assert!(!is_authorized(
            &EndpointMode::RemotePublic,
            consent_none,
            fp
        ));
        assert!(!is_authorized(
            &EndpointMode::PrivateNetwork,
            consent_none,
            fp
        ));

        let consent_remote = AiEndpointConsent {
            endpoint_fingerprint: fp.to_string(),
            remote_approved: true,
            private_network_approved: false,
            policy_version: 1,
            updated_at_ms: 0,
        };

        assert!(is_authorized(
            &EndpointMode::RemotePublic,
            Some(&consent_remote),
            fp
        ));
        assert!(!is_authorized(
            &EndpointMode::PrivateNetwork,
            Some(&consent_remote),
            fp
        ));

        let consent_both = AiEndpointConsent {
            endpoint_fingerprint: fp.to_string(),
            remote_approved: true,
            private_network_approved: true,
            policy_version: 1,
            updated_at_ms: 0,
        };

        assert!(is_authorized(
            &EndpointMode::PrivateNetwork,
            Some(&consent_both),
            fp
        ));
    }
}
