use bytes::{BufMut, Bytes, BytesMut};
use futures::stream::Stream;
use futures::AsyncRead;
use pin_project_lite::pin_project;
use std::io;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::{Context, Poll};

const DEFAULT_CAPACITY: usize = 4096;

pin_project! {
    #[derive(Debug)]
    pub(crate) struct ReaderStream<R> {
        #[pin]
        reader: Option<R>,
        buf: BytesMut,
        capacity: usize,
    }
}

#[inline(always)]
unsafe fn slice_assume_init_mut<T>(buf: &mut [MaybeUninit<T>]) -> &mut [T] {
    &mut *(buf as *mut [MaybeUninit<T>] as *mut [T])
}

fn poll_read_buf<T: AsyncRead + ?Sized, B: BufMut>(
    io: Pin<&mut T>,
    cx: &mut Context<'_>,
    buf: &mut B,
) -> Poll<io::Result<usize>> {
    if !buf.has_remaining_mut() {
        return Poll::Ready(Ok(0));
    }

    let n = {
        let dst = buf.chunk_mut();
        let buf = unsafe { slice_assume_init_mut(dst.as_uninit_slice_mut()) };
        match io.poll_read(cx, buf) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(n)) => n,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        }
    };

    unsafe {
        buf.advance_mut(n);
    }

    Poll::Ready(Ok(n))
}

impl<R: AsyncRead> ReaderStream<R> {
    pub fn new(reader: R) -> Self {
        ReaderStream {
            reader: Some(reader),
            buf: BytesMut::new(),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl<R: AsyncRead> Stream for ReaderStream<R> {
    type Item = std::io::Result<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        let reader = match this.reader.as_pin_mut() {
            Some(r) => r,
            None => return Poll::Ready(None),
        };

        if this.buf.capacity() == 0 {
            this.buf.reserve(*this.capacity);
        }

        match poll_read_buf(reader, cx, &mut this.buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(err)) => {
                self.project().reader.set(None);
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Ok(0)) => {
                self.project().reader.set(None);
                Poll::Ready(None)
            }
            Poll::Ready(Ok(_)) => {
                let chunk = this.buf.split();
                Poll::Ready(Some(Ok(chunk.freeze())))
            }
        }
    }
}
