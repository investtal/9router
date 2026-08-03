//! Stream helpers: pipe upstream bytes to the client without collecting the full body.
//!
//! **Invariant:** on the stream path we always use `reqwest::Response::bytes_stream()`
//! (or equivalent) and never `response.bytes().await` / `chunk()`-to-`Vec` aggregation.

use axum::body::Body;
use bytes::Bytes;
use futures_util::StreamExt;

/// Pipe upstream bytes to client without collecting the full body.
pub fn forward_byte_stream<S>(stream: S) -> Body
where
    S: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    Body::from_stream(stream.map(|r| r.map_err(|e| std::io::Error::other(e))))
}

/// Like [`forward_byte_stream`], but maps already-io-error-mapped chunks.
pub fn forward_io_stream<S>(stream: S) -> Body
where
    S: futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    Body::from_stream(stream)
}
