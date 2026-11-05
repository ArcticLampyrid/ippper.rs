#![cfg(feature = "server")]
use crate::service::IppService;
use crate::{body::Body, handler::handle_ipp_via_http};
use http::{Request, Response};
use hyper::{
    body::Incoming,
    service::{service_fn, Service},
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::error::Error as StdError;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
#[cfg(feature = "server-tls")]
use tokio_rustls::rustls::ServerConfig;

#[cfg(feature = "server-tls")]
mod adaptive;

/// Wrap an IPP service as a HTTP service
pub fn wrap_as_http_service<T>(
    ipp_service: Arc<T>,
) -> impl Service<
    Request<Incoming>,
    Response = Response<Body>,
    Error = anyhow::Error,
    Future = impl futures::Future<Output = Result<Response<Body>, anyhow::Error>> + 'static,
> + Clone
where
    T: IppService + 'static,
{
    service_fn(move |req| {
        let ipp_service = ipp_service.clone();
        async move { handle_ipp_via_http(req, ipp_service.as_ref()).await }
    })
}

/// Serve HTTP on the given address
pub async fn serve_http<S, B>(addr: SocketAddr, service: S) -> anyhow::Result<()>
where
    S: Service<Request<Incoming>, Response = Response<B>> + Clone + Send + 'static,
    S::Future: Send,
    S::Error: Into<Box<dyn StdError + Send + Sync>>,
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    let listener = TcpListener::bind(addr).await?;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                log::error!("Error accepting connection: {:?}", err);
                continue;
            }
        };
        let service = service.clone();
        tokio::task::spawn(async move {
            if let Err(err) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                log::error!("Error serving connection: {:?}", err);
            }
        });
    }
}

/// Serve HTTP and HTTPS on the same port.
///
/// Plain HTTP/1.1 clients may also switch to TLS in place, as described in
/// RFC 2817, by sending `Connection: Upgrade` and `Upgrade: TLS/1.2` or
/// `TLS/1.3`. The highest offered version enabled in `tls_config` is advertised
/// in the upgrade response; if none is available, the request is handled as
/// ordinary HTTP. The advertised version is advisory: the TLS handshake
/// independently negotiates a version allowed by `tls_config` and the client,
/// which may be higher or lower. After the handshake, the original request is
/// answered over TLS on HTTP/1.1.
///
/// The upgrade request, including its body, travels in plaintext. Clients that
/// need to protect a request should upgrade with `OPTIONS *` first. Upgrade
/// request bodies are buffered in temporary files without a size limit.
#[cfg(feature = "server-tls")]
pub async fn serve_adaptive_https<S, B>(
    addr: SocketAddr,
    service: S,
    tls_config: Arc<ServerConfig>,
) -> anyhow::Result<()>
where
    S: Service<Request<Incoming>, Response = Response<B>> + Clone + Send + 'static,
    S::Future: Send,
    S::Error: Into<Box<dyn StdError + Send + Sync>>,
    B: hyper::body::Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    let listener = TcpListener::bind(addr).await?;
    let acceptor = adaptive::Acceptor::new(tls_config).map_err(anyhow::Error::from_boxed)?;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                log::error!("Error accepting connection: {:?}", err);
                continue;
            }
        };
        let service = service.clone();
        let acceptor = acceptor.clone();
        tokio::task::spawn(async move {
            if let Err(err) = acceptor.serve(stream, service).await {
                log::error!("Error serving connection: {:?}", err);
            }
        });
    }
}

/// Create a TLS config from a reader of certificate and key files.  
/// ALPN protocols are automatically set to h2, http/1.1, and http/1.0.
/// TLS 1.2 and TLS 1.3 are enabled; TLS 1.0 and TLS 1.1 are not supported.
#[cfg(feature = "server-tls")]
pub fn tls_config_from_reader<R: std::io::Read>(cert: R, key: R) -> anyhow::Result<ServerConfig> {
    use std::io;
    use tokio_rustls::rustls::pki_types::{pem, pem::PemObject, CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_reader_iter(cert)
        .filter_map(|cert| cert.ok())
        .collect::<Vec<_>>();
    let key = match PrivateKeyDer::from_pem_reader(key) {
        Ok(x) => x,
        Err(pem::Error::NoItemsFound) => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "No private key found").into())
        }
        Err(e) => return Err(e.into()),
    };
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    Ok(config)
}
