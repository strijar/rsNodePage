//! Minimal NomadNetwork page-hosting node, built directly on rsReticulum's
//! `rns-runtime` Link/Request-Response layer.
//!
//! What this crate does *not* try to be: a full NomadNet client, a
//! propagation/LXMF node, or a renderer. It only serves the bytes behind
//! `/page/*.mu` and `/file/*` request paths over an RNS Link, the same way
//! the reference Python `nomadnet` node does.

pub mod announce;
pub mod config;
pub mod handler;
pub mod pages;
pub mod script;

pub use announce::APP_NAME;
pub use config::NomadConfig;
pub use pages::PageIndex;
