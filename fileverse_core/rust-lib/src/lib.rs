//! fileverse_core: a read-only viewer for Fileverse dDocs and dSheets that needs
//! no Fileverse server. A share link is resolved against the portal contract on
//! Gnosis and the IPFS blobs it names; every block is checked against its CID and
//! all decryption happens here.
//!
//! The engine (`abi`, `chain`, `ipfs`, `crypto`, `doc`, `sheet`, `viewer`) is plain
//! Rust, tested with `cargo test --no-default-features`; the Logos glue is behind
//! the default `logos_module` feature.

pub mod abi;
pub mod b64;
pub mod backup;
pub mod chain;
pub mod crypto;
pub mod doc;
pub mod ipfs;
pub mod md;
pub mod publish;
pub mod node;
pub mod sheet;
pub mod viewer;

#[cfg(feature = "logos_module")]
mod glue;
