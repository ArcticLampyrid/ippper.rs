use bytes::Buf;
use futures::task::Context;
use futures::task::Poll;
use futures::AsyncRead;
use http_body::Body as HttpBody;
use std::io;
use std::pin::pin;
use std::pin::Pin;
pub(crate) struct BodyReader<ReqBody, ReqData, ReqError>
where
    ReqData: Buf + Sync + Unpin,
    ReqError: std::error::Error + Send + Sync,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError> + Unpin,
{
    body: ReqBody,
    chunk: Option<ReqData>,
}

impl<ReqBody, ReqData, ReqError> BodyReader<ReqBody, ReqData, ReqError>
where
    ReqData: Buf + Sync + Unpin,
    ReqError: std::error::Error + Send + Sync,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError> + Unpin,
{
    pub fn new(body: ReqBody) -> BodyReader<ReqBody, ReqData, ReqError> {
        BodyReader { body, chunk: None }
    }
}

impl<ReqBody, ReqData, ReqError> AsyncRead for BodyReader<ReqBody, ReqData, ReqError>
where
    ReqData: Buf + Sync + Unpin,
    ReqError: std::error::Error + Send + Sync,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        if let Some(mut data) = self.chunk.take() {
            let len = std::cmp::min(data.remaining(), buf.len());
            data.copy_to_slice(&mut buf[..len]);
            if data.has_remaining() {
                self.chunk.replace(data);
            }
            return Poll::Ready(Ok(len));
        }
        loop {
            match pin!(&mut self.body).poll_frame(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    if let Ok(mut data) = data.into_data() {
                        let len = std::cmp::min(data.remaining(), buf.len());
                        data.copy_to_slice(&mut buf[..len]);
                        if data.has_remaining() {
                            self.chunk.replace(data);
                        }
                        return Poll::Ready(Ok(len));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(Ok(0)),
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Error reading body: {}", e),
                    )))
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
