use crate::body::Body;
use crate::body_reader::BodyReader;
use crate::service::IppService;
use anyhow;
use bytes::Buf;
use http::{Method, Request, Response};
use http_body::Body as HttpBody;
use ipp::parser::AsyncIppParser;
use std::sync::Arc;

pub async fn handle_ipp_via_http<ReqBody, ReqData, ReqError>(
    req: Request<ReqBody>,
    handler: Arc<impl IppService>,
) -> Result<Response<Body>, anyhow::Error>
where
    ReqData: Buf + Send + Sync + Unpin + 'static,
    ReqError: std::error::Error + Send + Sync + 'static,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError> + Send + Sync + Unpin + 'static,
{
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
    let (head, body) = req.into_parts();
    let reader = BodyReader::new(body);
    let ipp_request = AsyncIppParser::new(reader).parse().await?;
    let response = handler.handle_request(head, ipp_request).await;
    let body = Body::from(response);
    Ok(Response::builder()
        .status(200)
        .header("Content-Type", "application/ipp")
        .body(body)
        .unwrap())
}
