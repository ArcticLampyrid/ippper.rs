//! HTTP upgrade negotiation; application code runs only after TLS succeeds.

use super::{
    replay::CapturedRequest,
    tls::{TlsUpgrade, TlsVersion},
    BoxError,
};
use futures::future::{BoxFuture, FutureExt};
use http::{header, Request, Response, StatusCode};
use http_body_util::{Either, Empty};
use hyper::{
    body::{Body, Incoming},
    service::Service,
    upgrade::OnUpgrade,
};
use std::sync::{Arc, Mutex};

pub(super) struct PendingUpgrade {
    pub(super) request: CapturedRequest,
    pub(super) on_upgrade: OnUpgrade,
    pub(super) advertised_version: TlsVersion,
}

/// Hands an accepted upgrade from the plaintext connection back to its owner.
#[derive(Clone, Default)]
pub(super) struct UpgradeSlot(Arc<Mutex<Option<PendingUpgrade>>>);

impl UpgradeSlot {
    fn put(&self, pending: PendingUpgrade) {
        *self.0.lock().unwrap() = Some(pending);
    }

    pub(super) fn take(&self) -> Option<PendingUpgrade> {
        self.0.lock().unwrap().take()
    }
}

type UpgradeBody<B> = Either<Empty<<B as Body>::Data>, B>;

pub(super) struct UpgradeService<S> {
    inner: S,
    tls: Arc<TlsUpgrade>,
    slot: UpgradeSlot,
}

impl<S> UpgradeService<S> {
    pub(super) fn new(inner: S, tls: Arc<TlsUpgrade>, slot: UpgradeSlot) -> Self {
        Self { inner, tls, slot }
    }
}

impl<S, B> Service<Request<Incoming>> for UpgradeService<S>
where
    S: Service<Request<Incoming>, Response = Response<B>>,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    B: Body + 'static,
{
    type Response = Response<UpgradeBody<B>>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, mut request: Request<Incoming>) -> Self::Future {
        let Some(version) = self.tls.select(&request) else {
            let response = self.inner.call(request);
            return async move {
                let response = response.await.map_err(Into::into)?;
                Ok(response.map(Either::Right))
            }
            .boxed();
        };
        let on_upgrade = hyper::upgrade::on(&mut request);
        let slot = self.slot.clone();
        async move {
            // Polling Incoming lets Hyper issue 100 Continue if needed.
            // Do not send 101 until the whole plaintext body is captured.
            let request = CapturedRequest::capture(request).await?;
            slot.put(PendingUpgrade {
                request,
                on_upgrade,
                advertised_version: version,
            });
            Ok(Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .header(header::CONNECTION, "Upgrade")
                .header(header::UPGRADE, format!("{}, HTTP/1.1", version.to_token()))
                .body(Either::Left(Empty::new()))
                .unwrap())
        }
        .boxed()
    }
}
