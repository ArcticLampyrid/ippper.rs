use crate::utils::ReaderStream;
use bytes::Bytes;
use futures::stream::Stream;
use http_body::Body as HttpBody;
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use std::io;
use std::pin::pin;
use std::pin::Pin;
use std::task::{Context, Poll};
pub struct Body {
    pub(crate) inner: BodyInner,
}

pub(crate) enum BodyInner {
    Bytes(Option<Bytes>),
    IppRequestResponse {
        header: Option<Bytes>,
        payload: ReaderStream<IppPayload>,
    },
    Empty,
}

impl Body {
    /// Return an empty body.
    pub fn empty() -> Body {
        Body {
            inner: BodyInner::Empty,
        }
    }
}

impl Stream for Body {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.inner {
            BodyInner::Bytes(ref mut bytes) => Poll::Ready(bytes.take().map(Ok)),
            BodyInner::IppRequestResponse {
                ref mut header,
                ref mut payload,
            } => {
                if let Some(header) = header.take() {
                    Poll::Ready(Some(Ok(header)))
                } else {
                    pin!(payload).poll_next(cx)
                }
            }
            BodyInner::Empty => Poll::Ready(None),
        }
    }
}

impl HttpBody for Body {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.poll_next(cx).map_ok(http_body::Frame::data)
    }
}

impl From<String> for Body {
    fn from(t: String) -> Body {
        Body {
            inner: BodyInner::Bytes(Some(Bytes::from(t))),
        }
    }
}

impl From<&str> for Body {
    fn from(t: &str) -> Body {
        Body {
            inner: BodyInner::Bytes(Some(Bytes::from(t.to_string()))),
        }
    }
}

impl From<Bytes> for Body {
    fn from(t: Bytes) -> Body {
        Body {
            inner: BodyInner::Bytes(Some(t)),
        }
    }
}

impl From<IppRequestResponse> for Body {
    fn from(t: IppRequestResponse) -> Body {
        Body {
            inner: BodyInner::IppRequestResponse {
                header: Some(t.to_bytes()),
                payload: ReaderStream::new(t.into_payload()),
            },
        }
    }
}
