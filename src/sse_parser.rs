use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::stream::Stream;
use futures::StreamExt;

/// Parses a raw byte stream into SSE data payloads.
///
/// SSE events are lines of `data: ...` terminated by a blank line.
/// `SSEParser` aggregates those lines and yields each complete payload
/// as `Some(String)`.  The special `data: [DONE]` sentinel yields
/// `None`, which terminates the stream via `flat_map`.
struct SSEParser<S> {
    inner: S,
    acc: String,
}

impl<S> SSEParser<S>
where
    S: Stream<Item = Result<Bytes, String>>,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            acc: String::new(),
        }
    }

    /// Drain one complete SSE block from `self.acc` if available.
    fn poll_block(&mut self) -> Option<Option<String>> {
        if self.acc.starts_with("data: [DONE]") {
            self.acc.clear();
            return Some(None);
        }

        if let Some(pos) = self.acc.find("\n\n") {
            let block = self.acc[..pos].to_string();
            self.acc = self.acc[pos + 2..].to_string();

            let mut payload = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("data: ") {
                    if !payload.is_empty() {
                        payload.push('\n');
                    }
                    payload.push_str(rest);
                }
            }

            if payload.is_empty() {
                None
            } else {
                Some(Some(payload))
            }
        } else {
            None
        }
    }
}

impl<S> Stream for SSEParser<S>
where
    S: Stream<Item = Result<Bytes, String>> + Unpin,
{
    type Item = Option<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(block) = self.poll_block() {
                return Poll::Ready(Some(block));
            }

            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    self.acc.push_str(&String::from_utf8_lossy(&bytes));
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Some(format!("__SSE_ERROR__:{e}"))));
                }
                Poll::Ready(None) => {
                    if self.acc.is_empty() {
                        return Poll::Ready(None);
                    }
                    let remaining = std::mem::take(&mut self.acc);
                    if remaining.contains("data: [DONE]") {
                        return Poll::Ready(Some(None));
                    }
                    if let Some(pos) = remaining.find("data: ") {
                        let payload = remaining[pos + 6..].trim().to_string();
                        if !payload.is_empty() {
                            return Poll::Ready(Some(Some(payload)));
                        }
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Concrete stream type returned by both providers' `send_streaming`.
///
/// Wraps the raw HTTP byte stream and provider-specific message
/// extraction into a single `Stream<Item = Result<String, E>>`.
/// The `Pin` used internally for the raw bytes is fully encapsulated;
/// callers interact with a concrete `ModelStream<E>` that is `Unpin`.
pub struct ModelStream<E> {
    inner: Pin<Box<dyn Stream<Item = Result<String, E>> + Send>>,
}

impl<E> Unpin for ModelStream<E> {}

impl<E: 'static + Send> ModelStream<E> {
    /// Build a `ModelStream` from an HTTP byte stream and a provider-specific
    /// parse function.
    ///
    /// `byte_stream` is the raw HTTP byte stream with `reqwest::Error` already
    /// mapped to `String`.
    /// `parse_fn` receives a complete SSE data payload and extracts text chunks.
    /// `err_map` converts raw SSE parse errors into the provider's error type `E`.
    pub fn new(
        byte_stream: Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>,
        parse_fn: fn(&str) -> Result<Vec<String>, String>,
        err_map: fn(String) -> E,
    ) -> Self {
        // 1. Parse raw bytes into SSE payloads (Option<String>).
        let sse = SSEParser::new(byte_stream);

        // 2. For each payload, run the provider's parse function and yield
        //    individual text chunks, filtering empties.
        //    An SSE byte-stream error is surfaced via err_map.
        //    [DONE] (None) terminates the inner flat_map stream.
        let chunks = sse.flat_map(move |payload| -> Pin<Box<dyn Stream<Item = Result<String, E>> + Send>> {
            match payload {
                None => Box::pin(futures::stream::empty()), // [DONE]
                Some(s) if s.starts_with("__SSE_ERROR__:") => {
                    let msg = s.trim_start_matches("__SSE_ERROR__:").to_string();
                    Box::pin(futures::stream::once(async move { Err(err_map(msg)) }))
                }
                Some(json_str) => {
                    let texts: Vec<Result<String, E>> = match parse_fn(&json_str) {
                        Ok(t) => t.into_iter().filter(|s| !s.is_empty()).map(Ok).collect(),
                        Err(_) => vec![],
                    };
                    Box::pin(futures::stream::iter(texts))
                }
            }
        });

        Self {
            inner: Box::pin(chunks),
        }
    }
}

impl<E> Stream for ModelStream<E> {
    type Item = Result<String, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safe because ModelStream is Unpin (the inner Pin<Box> is always Unpin).
        Pin::new(&mut self.get_mut().inner).poll_next(cx)
    }
}
