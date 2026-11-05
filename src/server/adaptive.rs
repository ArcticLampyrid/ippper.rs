//! Serve plain HTTP and TLS on one port, including RFC 2817 in-place upgrades.

mod replay;
mod tls;
mod upgrade;

use http::{Request, Response};
use hyper::{
    body::{Body, Incoming},
    server::conn::http1,
    service::Service,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
};
use std::{error::Error, io::Cursor, sync::Arc};
use tokio::{
    io::{self, AsyncReadExt},
    net::TcpStream,
};
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

use replay::{CapturedRequest, ReplayService};
use tls::TlsUpgrade;
use upgrade::{PendingUpgrade, UpgradeService, UpgradeSlot};

type BoxError = Box<dyn Error + Send + Sync>;

/// First byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 22;

#[derive(Clone)]
pub(super) struct Acceptor {
    direct: TlsAcceptor,
    upgrade: Arc<TlsUpgrade>,
}

impl Acceptor {
    pub(super) fn new(tls_config: Arc<ServerConfig>) -> Result<Self, BoxError> {
        Ok(Self {
            upgrade: Arc::new(TlsUpgrade::new(&tls_config)?),
            direct: TlsAcceptor::from(tls_config),
        })
    }

    pub(super) async fn serve<S, B>(
        &self,
        mut stream: TcpStream,
        service: S,
    ) -> Result<(), BoxError>
    where
        S: Service<Request<Incoming>, Response = Response<B>> + Clone + Send + 'static,
        S::Future: Send,
        S::Error: Into<BoxError>,
        B: Body + Send + 'static,
        B::Data: Send,
        B::Error: Into<BoxError>,
    {
        let mut header = [0u8; 1];
        if stream.read(&mut header).await? == 0 {
            return Ok(());
        }
        let (reader, writer) = stream.into_split();
        let stream = io::join(Cursor::new(header).chain(reader), writer);
        if header[0] == TLS_HANDSHAKE {
            let stream = self.direct.accept(stream).await?;
            return auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await;
        }

        // Keep the transition in this task so handshake errors and cancellation
        // retain the same lifetime as the connection.
        let slot = UpgradeSlot::default();
        let plain_service =
            UpgradeService::new(service.clone(), self.upgrade.clone(), slot.clone());
        auto::Builder::new(TokioExecutor::new())
            .serve_connection_with_upgrades(TokioIo::new(stream), plain_service)
            .await?;

        let Some(PendingUpgrade {
            request: CapturedRequest { parts, replay },
            on_upgrade,
            advertised_version,
        }) = slot.take()
        else {
            return Ok(());
        };
        // Upgraded includes Hyper's unread buffer, just as the h1 -> h2
        // transition rewinds buffered IO. An already-read ClientHello must
        // reach the TLS acceptor too.
        let stream = self
            .upgrade
            .accept(TokioIo::new(on_upgrade.await?), advertised_version)
            .await?;

        // RFC 2817 section 3.3: answer the ORIGINAL request after TLS.
        // Replay locally, never over the wire. Hyper retains responsibility
        // for response framing, streaming, HEAD semantics and keep-alive.
        let (reader, writer) = io::split(stream);
        let stream = io::join(replay.chain(reader), writer);
        http1::Builder::new()
            .serve_connection(TokioIo::new(stream), ReplayService::new(service, parts))
            .await?;
        Ok(())
    }
}
