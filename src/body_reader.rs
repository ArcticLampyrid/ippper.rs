use bytes::Buf;
use futures::task::Context;
use futures::task::Poll;
use futures::AsyncRead;
use http_body::Body as HttpBody;
use pin_project_lite::pin_project;
use std::io;
use std::pin::Pin;

pin_project! {
    pub(crate) struct BodyReader<ReqBody, ReqData> {
        #[pin]
        body: ReqBody,
        chunk: Option<ReqData>,
    }
}

impl<ReqBody, ReqData, ReqError> BodyReader<ReqBody, ReqData>
where
    ReqData: Buf + Sync,
    ReqError: std::error::Error + Send + Sync,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError>,
{
    pub fn new(body: ReqBody) -> BodyReader<ReqBody, ReqData> {
        BodyReader { body, chunk: None }
    }
}

impl<ReqBody, ReqData, ReqError> AsyncRead for BodyReader<ReqBody, ReqData>
where
    ReqData: Buf + Sync,
    ReqError: std::error::Error + Send + Sync,
    ReqBody: HttpBody<Data = ReqData, Error = ReqError>,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        let mut this = self.as_mut().project();

        if let Some(mut data) = this.chunk.take() {
            let len = std::cmp::min(data.remaining(), buf.len());
            data.copy_to_slice(&mut buf[..len]);
            if data.has_remaining() {
                this.chunk.replace(data);
            }
            return Poll::Ready(Ok(len));
        }

        loop {
            match this.body.as_mut().poll_frame(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    if let Ok(mut data) = data.into_data() {
                        let len = std::cmp::min(data.remaining(), buf.len());
                        data.copy_to_slice(&mut buf[..len]);
                        if data.has_remaining() {
                            this.chunk.replace(data);
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
