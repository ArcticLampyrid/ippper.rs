use crate::body::Body;
use crate::body_reader::BodyReader;
use crate::service::IppService;
use anyhow;
use bytes::Buf;
use http::{Method, Request, Response, StatusCode};
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
    if req.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header("Allow", "POST")
            .body(Body::from("405 Method Not Allowed"))
            .unwrap());
    }
    if req.headers().get("Content-Type") != Some(&"application/ipp".parse().unwrap()) {
        return Ok(Response::builder()
            .status(StatusCode::UNSUPPORTED_MEDIA_TYPE)
            .body(Body::from("415 Unsupported Media Type"))
            .unwrap());
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
