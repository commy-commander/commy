use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

const LOOK_TIMEOUT: Duration = Duration::from_secs(6);
pub const MAX_PEM_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub issuer: String,
    pub not_before: i64,
    pub not_after: i64,
    pub names: Vec<String>,
}

impl Facts {
    pub fn covers(&self, domain: &str) -> bool {
        self.names.iter().any(|name| name_covers(name, domain))
    }
}

pub fn name_covers(name: &str, domain: &str) -> bool {
    let (name, domain) = (name.to_ascii_lowercase(), domain.to_ascii_lowercase());
    match name.strip_prefix("*.") {
        Some(parent) => domain.split_once('.').is_some_and(|(label, rest)| !label.is_empty() && rest == parent),
        None => name == domain,
    }
}

pub fn read(der: &[u8]) -> Result<Facts, String> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).map_err(|_| "the certificate could not be read".to_string())?;
    let issuer = cert.issuer();
    let issuer = issuer
        .iter_organization()
        .chain(issuer.iter_common_name())
        .find_map(|attribute| attribute.as_str().ok())
        .unwrap_or("an unnamed issuer")
        .to_string();
    let mut names: Vec<String> = match cert.subject_alternative_name() {
        Ok(Some(extension)) => extension
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                x509_parser::extensions::GeneralName::DNSName(dns) => Some(dns.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    if names.is_empty() {
        names.extend(cert.subject().iter_common_name().filter_map(|a| a.as_str().ok()).map(String::from));
    }
    Ok(Facts { issuer, not_before: cert.validity().not_before.timestamp(), not_after: cert.validity().not_after.timestamp(), names })
}

pub fn check_pair(certificate: &str, key: &str, domain: Option<&str>, now_s: i64) -> Result<Facts, String> {
    if certificate.len() > MAX_PEM_BYTES || key.len() > MAX_PEM_BYTES {
        return Err("that file is too large to be a certificate".into());
    }
    let chain = CertificateDer::pem_slice_iter(certificate.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "the certificate is not in PEM format (it should start with -----BEGIN CERTIFICATE-----)".to_string())?;
    let leaf = chain.first().ok_or("paste the certificate, starting with -----BEGIN CERTIFICATE-----")?;
    let facts = read(leaf)?;
    let private = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|_| "the private key is not in PEM format (it should start with -----BEGIN PRIVATE KEY-----)".to_string())?;
    rustls::sign::CertifiedKey::from_der(chain.clone(), private, &rustls::crypto::ring::default_provider())
        .map_err(|_| "the private key does not belong to this certificate".to_string())?;
    if facts.not_after <= now_s {
        return Err("this certificate has expired".into());
    }
    if facts.not_before > now_s + 60 {
        return Err("this certificate is not valid yet".into());
    }
    if let Some(domain) = domain {
        if !facts.covers(domain) {
            return Err(format!("this certificate is for {}, not {domain}", facts.names.join(", ")));
        }
    }
    Ok(facts)
}

pub async fn served(address: &str, domain: &str) -> Result<Facts, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Look(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(domain.to_string()).map_err(|_| format!("{domain} is not a domain name"))?;
    let look = async {
        let tcp = tokio::net::TcpStream::connect(address).await.map_err(|e| format!("could not reach the HTTPS server: {e}"))?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| format!("the HTTPS server has no certificate for {domain} yet ({e})"))?;
        let leaf = tls.get_ref().1.peer_certificates().and_then(|chain| chain.first().cloned()).ok_or("the HTTPS server sent no certificate")?;
        read(&leaf)
    };
    tokio::time::timeout(LOOK_TIMEOUT, look).await.map_err(|_| "the HTTPS server did not answer in time".to_string())?
}

#[derive(Debug)]
struct Look(Arc<CryptoProvider>);

impl ServerCertVerifier for Look {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(names: &[&str]) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
        (params.self_signed(&key).unwrap().pem(), key.serialize_pem())
    }

    #[test]
    fn names_cover_their_domain_and_wildcards_cover_one_label() {
        assert!(name_covers("sync.example.org", "Sync.Example.org"));
        assert!(name_covers("*.example.org", "sync.example.org"));
        assert!(!name_covers("*.example.org", "example.org"));
        assert!(!name_covers("*.example.org", "a.sync.example.org"));
        assert!(!name_covers("example.org", "sync.example.org"));
    }

    #[test]
    fn an_uploaded_pair_must_match_parse_and_cover_the_domain() {
        let now = 1_800_000_000;
        let (cert, key) = pair(&["sync.example.org"]);
        let facts = check_pair(&cert, &key, Some("sync.example.org"), now).unwrap();
        assert_eq!(facts.names, vec!["sync.example.org".to_string()]);
        assert!(check_pair(&cert, &key, Some("other.example.org"), now).unwrap_err().contains("not other.example.org"));
        let (_, other_key) = pair(&["sync.example.org"]);
        assert!(check_pair(&cert, &other_key, None, now).unwrap_err().contains("does not belong"));
        assert!(check_pair("hello", &key, None, now).is_err());
        assert!(check_pair(&cert, "hello", None, now).unwrap_err().contains("private key"));
        assert!(check_pair(&cert, &key, None, facts.not_after + 1).unwrap_err().contains("expired"));
    }
}
