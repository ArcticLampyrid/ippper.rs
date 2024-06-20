#![cfg(feature = "server")]
use crate::handler::handle_ipp_via_http;
use crate::service::IppService;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
pub async fn serve_ipp(
    addr: SocketAddr,
    ipp_service: Arc<impl IppService + 'static>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let ipp_service = ipp_service.clone();
        tokio::task::spawn(async move {
            if let Err(err) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let ipp_service = ipp_service.clone();
                        async move { handle_ipp_via_http(req, ipp_service).await }
                    }),
                )
                .await
            {
                log::error!("Error serving connection: {:?}", err);
            }
        });
    }
}
