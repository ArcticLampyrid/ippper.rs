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
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

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
        async move { handle_ipp_via_http(req, ipp_service).await }
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

/// Serve HTTP and HTTPS on the same port
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
    let acceptor = TlsAcceptor::from(tls_config);
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
            let mut header = [0u8; 1];
            if let Err(err) = stream.peek(&mut header).await {
                log::error!("Error peeking connection: {:?}", err);
                return;
            }
            let result = if header[0] != 22 {
                // Not a TLS connection
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service)
                    .await
            } else {
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        log::error!("Error accepting TLS connection: {:?}", err);
                        return;
                    }
                };
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service)
                    .await
            };
            if let Err(err) = result {
                log::error!("Error serving connection: {:?}", err);
            }
        });
    }
}

/// Create a TLS config from a reader of certificate and key files.  
/// ALPN protocols are automatically set to h2, http/1.1, and http/1.0.
#[cfg(feature = "server-tls")]
pub fn tls_config_from_reader<R: std::io::Read>(cert: R, key: R) -> anyhow::Result<ServerConfig> {
    use std::io::{self, BufReader};
    let certs = rustls_pemfile::certs(&mut BufReader::new(cert))
        .filter_map(|cert| cert.ok())
        .collect::<Vec<_>>();
    let key = rustls_pemfile::private_key(&mut BufReader::new(key))?;
    let key = match key {
        Some(x) => x,
        None => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "No private key found").into())
        }
    };
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    Ok(config)
}
