//! Capture an HTTP request without retaining its payload in memory, then
//! replay it into Hyper after the transport switches to TLS.

use http::{
    header::{self, HeaderName, HeaderValue},
    request::Parts,
    HeaderMap, Request,
};
use http_body_util::BodyExt;
use hyper::{body::Incoming, service::Service};
use std::{
    io::{self, Cursor},
    pin::Pin,
    sync::Mutex,
    task::{ready, Context, Poll},
};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, ReadBuf},
};

/// Release the spool at EOF, even while the chained connection stays alive.
pub(super) struct Replay(Option<Box<dyn AsyncRead + Unpin + Send>>);

impl AsyncRead for Replay {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if let Some(reader) = &mut self.0 {
            let before = buf.filled().len();
            ready!(Pin::new(reader).poll_read(cx, buf))?;
            if buf.filled().len() == before {
                self.0 = None;
            }
        }
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum CaptureError {
    #[error("invalid TLS upgrade request body: {0}")]
    Body(#[from] hyper::Error),
    #[error("TLS upgrade spool IO failed: {0}")]
    Io(#[from] io::Error),
    #[error("TLS upgrade spool task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

pub(super) struct CapturedRequest {
    /// Original request metadata, restored for the application by [`ReplayService`].
    pub(super) parts: Parts,
    /// The request re-encoded as HTTP/1.1 wire bytes, for Hyper to parse again.
    pub(super) replay: Replay,
}

impl CapturedRequest {
    pub(super) async fn capture(request: Request<Incoming>) -> Result<Self, CaptureError> {
        let (parts, body) = request.into_parts();
        let replay = SpooledBody::read(body).await?.into_replay(&parts).await?;
        Ok(Self { parts, replay })
    }
}

/// A request body's payload on disk plus its trailers. Wire framing (chunk
/// sizes and extensions) is dropped, which avoids disk amplification from
/// many tiny chunks.
#[derive(Default)]
struct SpooledBody {
    file: Option<File>,
    size: u64,
    trailers: Option<HeaderMap>,
}

impl SpooledBody {
    async fn read(mut body: Incoming) -> Result<Self, CaptureError> {
        let mut spooled = Self::default();
        while let Some(frame) = body.frame().await {
            match frame?.into_data() {
                Ok(data) => spooled.write(&data).await?,
                Err(frame) => {
                    if let Ok(trailers) = frame.into_trailers() {
                        spooled.trailers = Some(trailers);
                    }
                }
            }
        }
        Ok(spooled)
    }

    async fn write(&mut self, data: &[u8]) -> Result<(), CaptureError> {
        if data.is_empty() {
            return Ok(());
        }
        let file = match &mut self.file {
            Some(file) => file,
            // Anonymous/private temporary files are deleted when their
            // handles close, including on errors/cancellation.
            None => {
                let spool = tokio::task::spawn_blocking(tempfile::tempfile).await??;
                self.file.insert(File::from_std(spool))
            }
        };
        file.write_all(data).await?;
        self.size += data.len() as u64;
        Ok(())
    }

    async fn into_replay(self, parts: &Parts) -> Result<Replay, CaptureError> {
        let chunked = parts.headers.contains_key(header::TRANSFER_ENCODING);
        let mut head = request_head(parts);
        let mut tail = Vec::new();
        if chunked {
            // On replay, the whole payload forms a single chunk.
            if self.size > 0 {
                head.extend_from_slice(format!("{:x}\r\n", self.size).as_bytes());
                tail.extend_from_slice(b"\r\n");
            }
            tail.extend_from_slice(b"0\r\n");
            append_headers(&mut tail, self.trailers.iter().flatten());
            tail.extend_from_slice(b"\r\n");
        }

        let reader: Box<dyn AsyncRead + Unpin + Send> = match self.file {
            Some(mut file) => {
                // No durability sync is needed for this transient spool; flush
                // only surfaces pending write errors before reading back.
                file.flush().await?;
                file.rewind().await?;
                Box::new(Cursor::new(head).chain(file).chain(Cursor::new(tail)))
            }
            None => Box::new(Cursor::new([head, tail].concat())),
        };
        Ok(Replay(Some(reader)))
    }
}

fn request_head(parts: &Parts) -> Vec<u8> {
    let mut head = format!("{} {} HTTP/1.1\r\n", parts.method, parts.uri).into_bytes();
    // Expect was already handled on the plaintext connection. Replaying it
    // would cause a second 100 Continue after TLS. ReplayService restores the
    // original headers for application code without affecting Hyper's parser.
    let headers = parts
        .headers
        .iter()
        .filter(|(name, _)| *name != header::EXPECT);
    append_headers(&mut head, headers);
    head.extend_from_slice(b"\r\n");
    head
}

fn append_headers<'a>(
    output: &mut Vec<u8>,
    headers: impl IntoIterator<Item = (&'a HeaderName, &'a HeaderValue)>,
) {
    for (name, value) in headers {
        output.extend_from_slice(name.as_str().as_bytes());
        output.extend_from_slice(b": ");
        output.extend_from_slice(value.as_bytes());
        output.extend_from_slice(b"\r\n");
    }
}

/// Restore the original request's metadata once, while retaining the body
/// driven by the new Hyper connection. Later requests pass through unchanged.
pub(super) struct ReplayService<S> {
    inner: S,
    original: Mutex<Option<Parts>>,
}

impl<S> ReplayService<S> {
    pub(super) fn new(inner: S, original: Parts) -> Self {
        Self {
            inner,
            original: Mutex::new(Some(original)),
        }
    }
}

impl<S: Service<Request<Incoming>>> Service<Request<Incoming>> for ReplayService<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let original = self.original.lock().unwrap().take();
        let request = match original {
            Some(parts) => Request::from_parts(parts, request.into_body()),
            None => request,
        };
        self.inner.call(request)
    }
}
