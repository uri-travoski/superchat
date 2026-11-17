//! X-01 / AD-10: TLS with certificate validation, and trust-on-first-use pinning for servers
//! with a self-signed certificate. Nothing is ever accepted silently: the app first calls
//! [`probe_certificate`], shows the SHA-256 fingerprint, and only after the user confirms
//! stores it as the pin.

use crate::error::{ClientError, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn normalise_fp(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_uppercase()
}

fn root_store() -> Arc<RootCertStore> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(roots)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts a certificate if it chains to a public root OR its SHA-256 fingerprint equals the pin.
#[derive(Debug)]
struct PinOrRoots {
    inner: Arc<WebPkiServerVerifier>,
    pin: Option<String>,
}

impl ServerCertVerifier for PinOrRoots {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        if let Some(pin) = &self.pin {
            if normalise_fp(&fingerprint(end_entity.as_ref())) == *pin {
                return Ok(ServerCertVerified::assertion());
            }
        }
        self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
    }

    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        // Pinned self-signed certs must still prove possession of the key.
        rustls::crypto::verify_tls12_signature(m, c, d, &provider().signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(m, c, d, &provider().signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider().signature_verification_algorithms.supported_schemes()
    }
}

/// rustls config used for REST and WebSocket traffic.
pub fn client_config(pin: Option<&str>) -> Arc<ClientConfig> {
    let inner = WebPkiServerVerifier::builder_with_provider(root_store(), provider())
        .build()
        .expect("webpki verifier");
    let verifier = Arc::new(PinOrRoots { inner, pin: pin.map(normalise_fp) });
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("tls versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(cfg)
}

/// Records whatever certificate the server presents without trusting it.
#[derive(Debug)]
struct Recorder {
    seen: Mutex<Option<Vec<u8>>>,
    inner: Arc<WebPkiServerVerifier>,
    chain_ok: Mutex<Option<bool>>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(end_entity.as_ref().to_vec());
        let ok = self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now).is_ok();
        *self.chain_ok.lock().unwrap_or_else(|e| e.into_inner()) = Some(ok);
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(m, c, d, &provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(m, c, d, &provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider().signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertProbe {
    /// SHA-256 of the leaf certificate, `AB:CD:...`.
    pub fingerprint: String,
    /// True when the certificate chains to a public root and matches the host (no pin needed).
    pub publicly_trusted: bool,
}

/// Connects to `host:port`, completes a TLS handshake without trusting the result and reports
/// the leaf certificate fingerprint so the user can verify and pin it (TOFU).
pub async fn probe_certificate(host: &str, port: u16) -> Result<CertProbe> {
    let inner = WebPkiServerVerifier::builder_with_provider(root_store(), provider())
        .build()
        .map_err(|e| ClientError::Local(e.to_string()))?;
    let rec = Arc::new(Recorder { seen: Mutex::new(None), inner, chain_ok: Mutex::new(None) });
    let cfg = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| ClientError::Local(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(rec.clone())
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(10), tokio::net::TcpStream::connect((host, port)))
        .await
        .map_err(|_| ClientError::Network("connection timed out".into()))?
        .map_err(|e| ClientError::Network(e.to_string()))?;
    let name = ServerName::try_from(host.to_string()).map_err(|e| ClientError::Local(e.to_string()))?;
    let _tls = connector
        .connect(name, tcp)
        .await
        .map_err(|e| ClientError::Network(format!("TLS handshake failed: {e}")))?;
    let cert = rec
        .seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| ClientError::Network("server sent no certificate".into()))?;
    let trusted = rec.chain_ok.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(false);
    Ok(CertProbe { fingerprint: fingerprint(&cert), publicly_trusted: trusted })
}
