//! The signing half and the wire codecs for putting stored pieces to an
//! s3-compatible object store: signer, request head writer, response head
//! parser and error body reader. Nothing here opens a connection.
pub mod error_body;
pub mod request;
pub mod response;
pub mod signer;
