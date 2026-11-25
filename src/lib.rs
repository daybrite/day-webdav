// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! `WebDAV` resource operations over [`day_part_http`].
//!
//! This crate owns URL confinement, DAV XML and bounded requests. Applications own sync state,
//! enrollment, conflicts and durable retry queues. Writes are sent once: a failed exchange can
//! have changed the server. Dropping an operation cancels its HTTP future but cannot undo remote
//! side effects. Error codes are developer diagnostics, not localized UI text.
mod client;
mod endpoint;
mod multistatus;
mod policy;
/// Isolated loopback fixture for the demo and transport tests; never a production DAV server.
#[cfg(all(feature = "demo-server", not(target_arch = "wasm32")))]
pub mod testing;
pub use client::{
    Authentication, Client, ClientBuilder, Condition, ConnectionFailure, ConnectionStep, Depth,
    Discovery, EntityTag, RangeDownload,
};
pub use day_part_http as http;
pub use endpoint::{Endpoint, ResourcePath};
pub use multistatus::{MultiStatus, Property, PropertyName, PropertyStatus, Resource, XmlLimits};
pub use policy::{NetworkState, Policy, Retry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    /// No mutating request was attempted (a safe read may have been transmitted).
    NotSent,
    /// Reconcile the intended resource before deciding whether to repeat the operation.
    MayHaveApplied,
    /// A complete DAV response describes individual resource/property outcomes.
    MultiStatus,
}
/// Structured diagnostics. Do not render Debug/Display directly as application UI.
#[derive(Debug)]
pub enum Error {
    InvalidEndpoint,
    InvalidPath,
    UnsafeHref,
    InvalidEntityTag,
    Xml,
    Limit,
    Deferred(NetworkState),
    Transport {
        source: http::HttpError,
        outcome: WriteOutcome,
    },
    Status {
        status: u16,
        retry_after: Option<std::time::Duration>,
        outcome: WriteOutcome,
    },
    Redirect {
        status: u16,
        outcome: WriteOutcome,
    },
    Partial(MultiStatus),
    Protocol,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidEndpoint => "dav.invalid_endpoint",
            Self::InvalidPath => "dav.invalid_path",
            Self::UnsafeHref => "dav.unsafe_href",
            Self::InvalidEntityTag => "dav.invalid_etag",
            Self::Xml => "dav.invalid_xml",
            Self::Limit => "dav.limit",
            Self::Deferred(_) => "dav.deferred",
            Self::Transport { .. } => "dav.transport",
            Self::Status { .. } => "dav.status",
            Self::Redirect { .. } => "dav.redirect",
            Self::Partial(_) => "dav.partial",
            Self::Protocol => "dav.protocol",
        })
    }
}
impl Error {
    /// A redacted diagnostic identifier. Excludes URLs, credentials and server error text.
    pub fn diagnostic_code(&self) -> String {
        use http::{HttpError as H, NetworkRestriction as N};
        match self {
            Self::Transport { source, .. } => match source {
                H::NetworkRestricted(N::Expensive) => "network.expensive",
                H::NetworkRestricted(N::Constrained) => "network.constrained",
                H::NetworkRestricted(N::Cellular) => "network.cellular",
                H::Dns => "http.dns",
                H::Connect => "http.connect",
                H::Timeout => "http.timeout",
                H::Tls(_) => "http.tls",
                H::Io(_) => "http.io",
                H::BadUrl(_) => "http.bad_url",
                H::Cancelled => "http.cancelled",
                H::Unsupported => "http.unsupported",
                H::BodyTooLarge { .. } => "http.body_too_large",
                _ => "dav.transport",
            },
            Self::Deferred(NetworkState::Expensive) => "network.expensive",
            Self::Deferred(NetworkState::Constrained) => "network.constrained",
            Self::Deferred(NetworkState::Offline) => "network.offline",
            _ => return self.to_string(),
        }
        .into()
    }
}

impl std::error::Error for Error {}
