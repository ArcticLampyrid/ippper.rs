//! TLS upgrade token selection and configured-version discovery.

use super::BoxError;
use http::{header, HeaderMap, Request, Version};
use std::{io, sync::Arc};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{
    rustls::{
        self,
        server::{ClientHello, ResolvesServerCert},
        sign::CertifiedKey,
        ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
        SupportedProtocolVersion,
    },
    server::TlsStream,
    TlsAcceptor,
};

/// TLS versions an upgrade may offer, ordered by preference. rustls cannot
/// implement TLS 1.0 or 1.1, so those tokens are never acknowledged.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TlsVersion {
    Tls12,
    Tls13,
}

impl TlsVersion {
    const ALL: [Self; 2] = [Self::Tls12, Self::Tls13];

    fn parse_token(token: &str) -> Option<Self> {
        let (protocol, version) = token.split_once('/')?;
        if !protocol.eq_ignore_ascii_case("tls") {
            return None;
        }
        match version {
            "1.2" => Some(Self::Tls12),
            "1.3" => Some(Self::Tls13),
            _ => None,
        }
    }

    fn rustls(self) -> &'static SupportedProtocolVersion {
        match self {
            Self::Tls12 => &rustls::version::TLS12,
            Self::Tls13 => &rustls::version::TLS13,
        }
    }

    pub(super) fn to_token(self) -> &'static str {
        match self {
            Self::Tls12 => "TLS/1.2",
            Self::Tls13 => "TLS/1.3",
        }
    }
}

pub(super) struct TlsUpgrade {
    acceptor: TlsAcceptor,
    versions: Vec<TlsVersion>,
}

impl TlsUpgrade {
    pub(super) fn new(config: &ServerConfig) -> Result<Self, BoxError> {
        // RFC 2817 retains HTTP/1.1, even if direct HTTPS supports h2 via ALPN.
        let mut config = config.clone();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let versions = TlsVersion::ALL
            .into_iter()
            .map(|version| Ok(supports_version(&config, version.rustls())?.then_some(version)))
            .filter_map(Result::transpose)
            .collect::<Result<_, BoxError>>()?;
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(config)),
            versions,
        })
    }

    /// Select the advisory version advertised in the upgrade response, not a
    /// constraint on the TLS handshake. Unknown versions and versions disabled
    /// in the config are ignored; the highest available offer is advertised.
    pub(super) fn select<B>(&self, request: &Request<B>) -> Option<TlsVersion> {
        let headers = request.headers();
        let wants_upgrade = request.version() == Version::HTTP_11
            && tokens(headers, header::CONNECTION)
                .any(|token| token.eq_ignore_ascii_case("upgrade"));
        if !wants_upgrade {
            return None;
        }
        tokens(headers, header::UPGRADE)
            .filter_map(TlsVersion::parse_token)
            .filter(|version| self.versions.contains(version))
            .max()
    }

    pub(super) async fn accept<IO>(
        &self,
        stream: IO,
        advertised_version: TlsVersion,
    ) -> io::Result<TlsStream<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        // The HTTP upgrade token is advisory by design. Keep every configured
        // TLS version available and let the handshake negotiate independently;
        // a higher or lower version than advertised is not an error.
        let stream = self.acceptor.accept(stream).await?;
        log::debug!(
            "TLS upgrade completed: advertised {:?}, negotiated {:?} (advertised version is advisory)",
            advertised_version.rustls().version,
            stream.get_ref().1.protocol_version(),
        );
        Ok(stream)
    }
}

fn tokens(headers: &HeaderMap, name: header::HeaderName) -> impl Iterator<Item = &str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim_matches([' ', '\t']))
}

/// rustls 0.23 does not expose ServerConfig's enabled protocol versions.
/// Probe each version once at listener setup using an in-memory ClientHello.
/// Looking only at the provider's cipher suites would incorrectly accept
/// versions that the caller disabled with with_protocol_versions().
fn supports_version(
    config: &ServerConfig,
    version: &'static SupportedProtocolVersion,
) -> Result<bool, BoxError> {
    if !config
        .crypto_provider()
        .cipher_suites
        .iter()
        .any(|suite| suite.version().version == version.version)
    {
        return Ok(false);
    }

    let mut client_config = ClientConfig::builder_with_provider(config.crypto_provider().clone())
        .with_protocol_versions(&[version])?
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    client_config.enable_sni = false;
    let mut client = ClientConnection::new(Arc::new(client_config), "localhost".try_into()?)?;
    let mut hello = Vec::new();
    client.write_tls(&mut hello)?;

    // Stop before certificate selection/signing. Never invoke the caller's
    // certificate resolver or perform a real handshake during discovery.
    let mut probe_config = config.clone();
    probe_config.cert_resolver = Arc::new(NoCertificate);
    let mut server = ServerConnection::new(Arc::new(probe_config))?;
    server.read_tls(&mut hello.as_slice())?;
    let _ = server.process_new_packets();
    // rustls records the negotiated version before resolving a certificate.
    // Disabled versions are rejected earlier and leave this unset.
    Ok(server.protocol_version() == Some(version.version))
}

#[derive(Debug)]
struct NoCertificate;

impl ResolvesServerCert for NoCertificate {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        None
    }
}
