use crate::error::IppError;
use crate::result::IppResult;
use anyhow;
use async_trait::async_trait;
use futures::stream::TryStreamExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::Method;
use hyper::{Body, Request, Response, Server};
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, IppVersion, Operation, StatusCode};
use ipp::parser::AsyncIppParser;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use num_traits::FromPrimitive;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::compat::*;
use tokio_util::io::ReaderStream;

fn operation_not_supported() -> anyhow::Error {
    anyhow::Error::new(IppError {
        code: StatusCode::ServerErrorOperationNotSupported,
        msg: StatusCode::ServerErrorOperationNotSupported.to_string(),
    })
}

#[async_trait]
pub trait IppServerHandler: Send + Sync + 'static {
    async fn print_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn print_uri(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn validate_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn create_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn send_document(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn send_uri(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn cancel_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_job_attributes(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_jobs(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_printer_attributes(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn hold_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn release_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn restart_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn pause_printer(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn resume_printer(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn purge_jobs(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    fn version(&self) -> IppVersion {
        IppVersion::v1_1()
    }

    fn check_version(&self, req: &IppRequestResponse) -> bool {
        let version = req.header().version.0;
        version <= self.version().0
    }

    fn build_error_response(
        &self,
        version: IppVersion,
        req_id: u32,
        error: anyhow::Error,
    ) -> IppRequestResponse {
        let ipp_error = match error.downcast_ref::<IppError>() {
            Some(e) => e.clone(),
            None => IppError {
                code: StatusCode::ServerErrorInternalError,
                msg: error.to_string(),
            },
        };
        let mut resp = IppRequestResponse::new_response(version, ipp_error.code, req_id);
        resp.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::new(
                IppAttribute::STATUS_MESSAGE,
                IppValue::TextWithoutLanguage(ipp_error.msg),
            ),
        );
        resp
    }

    async fn handle_request(&self, req: IppRequestResponse) -> IppRequestResponse {
        let req_id = req.header().request_id;
        if !self.check_version(&req) {
            return self.build_error_response(
                self.version(),
                req_id,
                IppError {
                    code: StatusCode::ServerErrorVersionNotSupported,
                    msg: StatusCode::ServerErrorVersionNotSupported.to_string(),
                }
                .into(),
            );
        }
        let version = req.header().version;
        match Operation::from_u16(req.header().operation_or_status) {
            Some(op) => match op {
                Operation::PrintJob => self.print_job(req).await,
                Operation::PrintUri => self.print_uri(req).await,
                Operation::ValidateJob => self.validate_job(req).await,
                Operation::CreateJob => self.create_job(req).await,
                Operation::SendDocument => self.send_document(req).await,
                Operation::SendUri => self.send_uri(req).await,
                Operation::CancelJob => self.cancel_job(req).await,
                Operation::GetJobAttributes => self.get_job_attributes(req).await,
                Operation::GetJobs => self.get_jobs(req).await,
                Operation::GetPrinterAttributes => self.get_printer_attributes(req).await,
                Operation::HoldJob => self.hold_job(req).await,
                Operation::ReleaseJob => self.release_job(req).await,
                Operation::RestartJob => self.restart_job(req).await,
                Operation::PausePrinter => self.pause_printer(req).await,
                Operation::ResumePrinter => self.resume_printer(req).await,
                Operation::PurgeJobs => self.purge_jobs(req).await,
                _ => Err(operation_not_supported()),
            },
            None => Err(operation_not_supported()),
        }
        .unwrap_or_else(|error| self.build_error_response(version, req_id, error))
    }
}

pub struct IppServer {}

impl IppServer {
    pub async fn serve(
        addr: SocketAddr,
        handler: Arc<impl IppServerHandler>,
    ) -> std::result::Result<(), hyper::Error> {
        let addr = addr;
        let make_svc = make_service_fn(|_conn| {
            let handler = handler.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let handler = handler.clone();
                    Self::handle(req, handler)
                }))
            }
        });
        let server = Server::bind(&addr).serve(make_svc);
        server.await
    }

    async fn handle(
        req: Request<Body>,
        handler: Arc<impl IppServerHandler>,
    ) -> Result<Response<Body>, anyhow::Error> {
        if req.method() == Method::GET {
            return match req.uri().path() {
                "/" => Ok(Response::builder()
                    .status(200)
                    .body(Body::from("IPP server running..."))
                    .unwrap()),
                _ => Ok(Response::builder()
                    .status(404)
                    .body(Body::from("404 Not Found"))
                    .unwrap()),
            };
        }
        let reader = req
            .into_body()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
            .into_async_read();
        let ipp_request = AsyncIppParser::new(reader).parse().await?;
        let response = handler.handle_request(ipp_request).await;
        let body = Body::wrap_stream(ReaderStream::new(response.into_async_read().compat()));
        Ok(Response::builder()
            .status(200)
            .header("Content-Type", "application/ipp")
            .body(body)
            .unwrap())
    }
}
