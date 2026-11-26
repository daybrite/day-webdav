// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use crate::{
    Endpoint, Error, MultiStatus, NetworkState, Policy, ResourcePath, WriteOutcome, XmlLimits, http,
};
use http::{
    Cache, CachePolicy, Cookies, Method, Redirects, Request, Response, Session,
    simulation::{Clock, RealClock},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

const PROPERTIES: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/><d:getetag/><d:getcontentlength/><d:getlastmodified/><d:displayname/><d:supported-report-set/></d:prop></d:propfind>";

#[derive(Clone, Default)]
pub enum Authentication {
    #[default]
    None,
    Basic {
        username: String,
        password: String,
    },
    Bearer(String),
}
impl std::fmt::Debug for Authentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authentication([redacted])")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityTag(String);
impl EntityTag {
    /// Parse an `ETag` header value.
    ///
    /// # Errors
    /// Returns `Error::InvalidEntityTag` when `value` is not a quoted entity tag, weak or strong.
    pub fn parse(value: &str) -> Result<Self, Error> {
        let raw = value.strip_prefix("W/").unwrap_or(value);
        if raw.len() < 2
            || !raw.starts_with('"')
            || !raw.ends_with('"')
            || !raw[1..raw.len() - 1]
                .bytes()
                .all(|b| b == 0x21 || (0x23..=0x7e).contains(&b) || b >= 0x80)
        {
            return Err(Error::InvalidEntityTag);
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_weak(&self) -> bool {
        self.0.starts_with("W/")
    }
}
#[derive(Clone, Debug, Default)]
pub enum Condition {
    #[default]
    Unconditional,
    CreateOnly,
    IfMatch(EntityTag),
}
#[derive(Clone, Copy, Debug)]
pub enum Depth {
    Zero,
    One,
}
#[derive(Clone, Debug)]
pub struct Discovery {
    pub dav: Vec<String>,
    pub allow: Vec<String>,
}
/// The operation that failed during an account's disposable write trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStep {
    CreateAppFolder,
    OpenCollection,
    CreateCollection,
    CreateFile,
    CheckCreateOnly,
    ListFiles,
    ReadFile,
    RemoveCollection,
}

#[derive(Debug)]
pub struct ConnectionFailure {
    pub step: ConnectionStep,
    pub source: Error,
}

impl ConnectionStep {
    /// DAV method used by this step of the account check.
    pub fn method(self) -> &'static str {
        match self {
            Self::OpenCollection | Self::ListFiles => "PROPFIND",
            Self::CreateAppFolder | Self::CreateCollection => "MKCOL",
            Self::CreateFile | Self::CheckCreateOnly => "PUT",
            Self::ReadFile => "GET",
            Self::RemoveCollection => "DELETE",
        }
    }

    fn failed(self, source: Error) -> ConnectionFailure {
        ConnectionFailure { step: self, source }
    }
}

/// Callers may append only Partial bytes, after validating their existing prefix separately.
#[derive(Debug)]
pub enum RangeDownload {
    Partial {
        response: Response,
        start: u64,
        end: u64,
        total: u64,
    },
    /// If-Range failed or Range was ignored. Replace the old representation; never append.
    Restart(Response),
}

pub struct ClientBuilder {
    endpoint: Endpoint,
    session: Session,
    auth: Authentication,
    policy: Policy,
    limits: XmlLimits,
    clock: Arc<dyn Clock>,
}
impl ClientBuilder {
    #[must_use]
    pub fn authentication(mut self, auth: Authentication) -> Self {
        self.auth = auth;
        self
    }
    #[must_use]
    pub fn session(mut self, session: Session) -> Self {
        self.session = session;
        self
    }
    #[must_use]
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }
    #[must_use]
    pub fn xml_limits(mut self, limits: XmlLimits) -> Self {
        self.limits = limits;
        self
    }
    /// Use the same `ManualClock` as HTTP simulation for deterministic fault/retry tests.
    #[must_use]
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
    pub fn build(self) -> Client {
        let http = http::Client::builder()
            .session(self.session)
            .redirects(Redirects::Never)
            .cookies(Cookies::Off)
            .cache(Cache::Off)
            .max_per_host(2)
            .wait_for_connectivity(false)
            .build();
        Client {
            inner: Arc::new(Inner {
                http,
                endpoint: self.endpoint,
                auth: self.auth,
                policy: self.policy,
                limits: self.limits,
                clock: self.clock,
                network: Mutex::new(NetworkState::Available),
                sequence: AtomicU64::new(0),
            }),
        }
    }
}
struct Inner {
    http: http::Client,
    endpoint: Endpoint,
    auth: Authentication,
    policy: Policy,
    limits: XmlLimits,
    clock: Arc<dyn Clock>,
    network: Mutex<NetworkState>,
    sequence: AtomicU64,
}
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}
impl Client {
    /// Run an enrollment trial inside a newly created, disposable child collection. Both
    /// create-only PUT and readback must work. Cleanup is attempted even after a failed trial.
    /// # Errors
    /// Returns transport, authentication, permission, cleanup, or compatibility errors.
    pub async fn check_writable_collection(&self) -> Result<(), Error> {
        self.check_writable_collection_detailed()
            .await
            .map_err(|failure| failure.source)
    }

    /// Run the enrollment trial and identify the failed operation for settings UIs.
    /// # Errors
    /// Returns the first failed check, or the cleanup error if all checks passed.
    pub async fn check_writable_collection_detailed(&self) -> Result<(), ConnectionFailure> {
        self.check_app_folder(false).await
    }

    /// Test this endpoint and, when requested, create its final collection if it is missing.
    /// The parent collection must already exist. Credential updates should pass `false`.
    ///
    /// # Errors
    /// Returns the failing operation and its DAV or transport error.
    pub async fn check_app_folder(&self, create_missing: bool) -> Result<(), ConnectionFailure> {
        use ConnectionStep as Step;
        match self.check_collection().await {
            Ok(()) => {}
            Err(Error::Status { status: 404, .. }) if create_missing => {
                // Another device may create the folder between PROPFIND and MKCOL.
                match self.mkcol(&ResourcePath::root()).await {
                    Ok(_) | Err(Error::Status { status: 405, .. }) => {}
                    Err(e) => return Err(Step::CreateAppFolder.failed(e)),
                }
                self.check_collection()
                    .await
                    .map_err(|e| Step::OpenCollection.failed(e))?;
            }
            Err(e) => return Err(Step::OpenCollection.failed(e)),
        }
        let name = format!("day-connection-test-{}", uuid::Uuid::new_v4());
        let dir = ResourcePath::from_segments([name.as_str()], true)
            .map_err(|e| Step::CreateCollection.failed(e))?;
        let file = ResourcePath::from_segments([name.as_str(), "probe"], false)
            .map_err(|e| Step::CreateFile.failed(e))?;
        self.mkcol(&dir)
            .await
            .map_err(|e| Step::CreateCollection.failed(e))?;
        let result =
            async {
                self.put(&file, name.as_bytes().to_vec(), Condition::CreateOnly)
                    .await
                    .map_err(|e| Step::CreateFile.failed(e))?;
                match self.put(&file, vec![0], Condition::CreateOnly).await {
                    Err(Error::Status { status: 412, .. }) => {}
                    Err(e) => return Err(Step::CheckCreateOnly.failed(e)),
                    Ok(_) => return Err(Step::CheckCreateOnly.failed(Error::Protocol)),
                }
                let listing = self
                    .propfind(&dir, Depth::One)
                    .await
                    .map_err(|e| Step::ListFiles.failed(e))?;
                if !listing.resources.iter().any(|r| {
                    r.href == self.endpoint().url(&file) && r.status.is_none_or(|s| s == 200)
                }) {
                    return Err(Step::ListFiles.failed(Error::Protocol));
                }
                let response = self
                    .get(&file, None)
                    .await
                    .map_err(|e| Step::ReadFile.failed(e))?;
                if response.body != name.as_bytes() {
                    return Err(Step::ReadFile.failed(Error::Protocol));
                }
                Ok(())
            }
            .await;
        let cleanup = self.delete(&dir, Condition::Unconditional).await;
        result?;
        cleanup
            .map(|_| ())
            .map_err(|e| Step::RemoveCollection.failed(e))
    }
    /// Authenticate to the configured collection and confirm that its own resource is readable.
    /// # Errors
    /// Returns the server or transport failure, or a protocol error for an incomplete response.
    pub async fn check_collection(&self) -> Result<(), Error> {
        let listing = self.propfind(&ResourcePath::root(), Depth::Zero).await?;
        let root = self.endpoint().as_str().trim_end_matches('/');
        if listing.resources.len() == 1
            && listing.resources.iter().all(|r| {
                r.href.trim_end_matches('/') == root
                    && r.is_collection()
                    && r.status.is_none_or(|s| s == 200)
            })
        {
            Ok(())
        } else {
            Err(Error::Protocol)
        }
    }

    pub fn builder(endpoint: Endpoint) -> ClientBuilder {
        ClientBuilder {
            endpoint,
            session: Session::global(),
            auth: Authentication::None,
            policy: Policy::default(),
            limits: XmlLimits::default(),
            clock: Arc::new(RealClock),
        }
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.inner.endpoint
    }
    /// Gates subsequent attempts. Drop in-flight futures to cancel already admitted transfers.
    pub fn set_network_state(&self, state: NetworkState) {
        *self
            .inner
            .network
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
    }
    fn admit(&self) -> Result<(), Error> {
        let state = *self
            .inner
            .network
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state == NetworkState::Offline
            || (state == NetworkState::Constrained && !self.inner.policy.allow_constrained)
            || (state == NetworkState::Expensive && !self.inner.policy.allow_expensive)
        {
            return Err(Error::Deferred(state));
        }
        Ok(())
    }
    /// Advertised tokens only; not a conformance certificate or atomic-write guarantee.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn options(&self) -> Result<Discovery, Error> {
        let r = self
            .exchange(Method::Options, &ResourcePath::root(), vec![], vec![], true)
            .await?;
        self.status(&r, &[200, 204], false)?;
        let values = |header: &str| {
            r.headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(header))
                .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_owned()))
                .filter(|s| !s.is_empty())
                .collect()
        };
        Ok(Discovery {
            dav: values("dav"),
            allow: values("allow"),
        })
    }
    /// Depth is deliberately bounded. A 404 property does not discard successful siblings.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept. A response that is not a bounded multistatus document within the endpoint is an
    /// error too.
    pub async fn propfind(&self, path: &ResourcePath, depth: Depth) -> Result<MultiStatus, Error> {
        let r = self
            .exchange(
                Method::Propfind,
                path,
                PROPERTIES.as_bytes().to_vec(),
                vec![
                    (
                        "Depth".into(),
                        match depth {
                            Depth::Zero => "0",
                            Depth::One => "1",
                        }
                        .into(),
                    ),
                    (
                        "Content-Type".into(),
                        "application/xml; charset=utf-8".into(),
                    ),
                ],
                true,
            )
            .await?;
        self.status(&r, &[207], false)?;
        let requested = self.endpoint().url(path);
        let result = MultiStatus::parse(&r.body, self.endpoint(), &requested, &self.inner.limits)?;
        let parent = url::Url::parse(&requested)
            .map_err(|_| Error::Protocol)?
            .path()
            .trim_end_matches('/')
            .to_owned();
        for resource in &result.resources {
            let url = url::Url::parse(&resource.href).map_err(|_| Error::Protocol)?;
            let resource_path = url.path().trim_end_matches('/');
            if resource_path != parent {
                let tail = resource_path
                    .strip_prefix(&format!("{parent}/"))
                    .ok_or(Error::UnsafeHref)?;
                if matches!(depth, Depth::Zero) || tail.contains('/') {
                    return Err(Error::UnsafeHref);
                }
            }
        }
        Ok(result)
    }
    /// Fetch a resource, conditionally when an entity tag is given.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn get(
        &self,
        path: &ResourcePath,
        if_none_match: Option<&EntityTag>,
    ) -> Result<Response, Error> {
        let headers = if_none_match.map_or_default(|t| vec![("If-None-Match".into(), t.0.clone())]);
        let r = self
            .exchange(Method::Get, path, vec![], headers, true)
            .await?;
        self.status(
            &r,
            if if_none_match.is_some() {
                &[200, 304]
            } else {
                &[200]
            },
            false,
        )?;
        Ok(r)
    }
    /// Fetch a resource's headers.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn head(&self, path: &ResourcePath) -> Result<Response, Error> {
        let r = self
            .exchange(Method::Head, path, vec![], vec![], true)
            .await?;
        self.status(&r, &[200], false)?;
        Ok(r)
    }
    /// Fetch a byte range of a resource.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn get_range(
        &self,
        path: &ResourcePath,
        start: u64,
        end: u64,
        validator: &EntityTag,
    ) -> Result<RangeDownload, Error> {
        if validator.is_weak() {
            return Err(Error::InvalidEntityTag);
        }
        let length = end
            .checked_sub(start)
            .and_then(|n| n.checked_add(1))
            .ok_or(Error::Limit)?;
        if length > self.inner.policy.max_response_bytes as u64 {
            return Err(Error::Limit);
        }
        let headers = vec![
            ("Range".into(), format!("bytes={start}-{end}")),
            ("If-Range".into(), validator.0.clone()),
        ];
        let r = self
            .exchange(Method::Get, path, vec![], headers, true)
            .await?;
        self.status(&r, &[200, 206], false)?;
        if r.status == 200 {
            return Ok(RangeDownload::Restart(r));
        }
        if r.header("content-encoding")
            .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
            || r.header("etag") != Some(validator.as_str())
        {
            return Err(Error::Protocol);
        }
        let range = r
            .header("content-range")
            .and_then(|v| v.strip_prefix("bytes "))
            .ok_or(Error::Protocol)?;
        let (span, total) = range.split_once('/').ok_or(Error::Protocol)?;
        let (a, b) = span.split_once('-').ok_or(Error::Protocol)?;
        let a: u64 = a.parse().map_err(|_| Error::Protocol)?;
        let b: u64 = b.parse().map_err(|_| Error::Protocol)?;
        let total: u64 = total.parse().map_err(|_| Error::Protocol)?;
        if a != start
            || b < a
            || b > end
            || total <= b
            || b != end.min(total - 1)
            || r.body.len() as u64 != b - a + 1
        {
            return Err(Error::Protocol);
        }
        Ok(RangeDownload::Partial {
            response: r,
            start: a,
            end: b,
            total,
        })
    }
    /// Create a collection.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn mkcol(&self, path: &ResourcePath) -> Result<Response, Error> {
        let r = self
            .exchange(Method::Mkcol, path, vec![], vec![], false)
            .await?;
        self.status(&r, &[201], true)?;
        Ok(r)
    }
    /// Store a resource's bytes under a condition.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept.
    pub async fn put(
        &self,
        path: &ResourcePath,
        bytes: Vec<u8>,
        condition: Condition,
    ) -> Result<Response, Error> {
        let mut headers = condition_headers(condition)?;
        headers.push(("Content-Type".into(), "application/octet-stream".into()));
        let r = self
            .exchange(Method::Put, path, bytes, headers, false)
            .await?;
        self.status(&r, &[200, 201, 204], true)?;
        Ok(r)
    }
    /// Resource/collection deletion is explicit; never inferred from missing listings.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept. A multistatus answer is `Error::Partial`.
    pub async fn delete(
        &self,
        path: &ResourcePath,
        condition: Condition,
    ) -> Result<Response, Error> {
        let r = self
            .exchange(
                Method::Delete,
                path,
                vec![],
                condition_headers(condition)?,
                false,
            )
            .await?;
        self.mutation_status(&r, path, &[200, 204])?;
        Ok(r)
    }
    /// COPY stays within the endpoint; overwriting a destination must be explicitly enabled.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept. A multistatus answer is `Error::Partial`.
    pub async fn copy(
        &self,
        from: &ResourcePath,
        to: &ResourcePath,
        overwrite: bool,
    ) -> Result<Response, Error> {
        self.relocate(Method::Copy, from, to, overwrite).await
    }
    /// Move a resource within the endpoint.
    ///
    /// # Errors
    /// Returns `Error::Deferred` when the network policy holds the request back, and otherwise an
    /// error when the transfer fails or the server answers with a status this method does not
    /// accept. A multistatus answer is `Error::Partial`.
    pub async fn move_resource(
        &self,
        from: &ResourcePath,
        to: &ResourcePath,
        overwrite: bool,
    ) -> Result<Response, Error> {
        self.relocate(Method::Move, from, to, overwrite).await
    }
    async fn relocate(
        &self,
        method: Method,
        from: &ResourcePath,
        to: &ResourcePath,
        overwrite: bool,
    ) -> Result<Response, Error> {
        let headers = vec![
            ("Destination".into(), self.endpoint().url(to)),
            ("Overwrite".into(), if overwrite { "T" } else { "F" }.into()),
        ];
        let r = self.exchange(method, from, vec![], headers, false).await?;
        self.mutation_status(&r, from, &[201, 204])?;
        Ok(r)
    }
    fn mutation_status(
        &self,
        r: &Response,
        path: &ResourcePath,
        accepted: &[u16],
    ) -> Result<(), Error> {
        if r.status == 207 {
            return Err(Error::Partial(MultiStatus::parse(
                &r.body,
                self.endpoint(),
                &self.endpoint().url(path),
                &self.inner.limits,
            )?));
        }
        self.status(r, accepted, true)
    }
    #[expect(
        clippy::unused_self,
        reason = "called as a method beside `mutation_status`, which reads the client"
    )]
    fn status(&self, r: &Response, accepted: &[u16], write: bool) -> Result<(), Error> {
        if accepted.contains(&r.status) {
            return Ok(());
        }
        let outcome = if write {
            WriteOutcome::MayHaveApplied
        } else {
            WriteOutcome::NotSent
        };
        if matches!(r.status, 301 | 302 | 303 | 307 | 308) {
            return Err(Error::Redirect {
                status: r.status,
                outcome,
            });
        }
        Err(Error::Status {
            status: r.status,
            retry_after: crate::policy::retry_after(
                r.header("retry-after"),
                crate::policy::wall_now(r),
            ),
            outcome,
        })
    }
    fn request(&self, method: Method, path: &ResourcePath) -> Request {
        let policy = &self.inner.policy;
        let request = Request::new(method, self.endpoint().url(path))
            .timeout(policy.idle_timeout)
            .allow_expensive(policy.allow_expensive)
            .allow_constrained(policy.allow_constrained)
            .cache(CachePolicy::Reload)
            .header("Accept-Encoding", "identity")
            .header("User-Agent", "day-webdav/0.1");
        match &self.inner.auth {
            Authentication::None => request,
            Authentication::Basic { username, password } => request.basic_auth(username, password),
            Authentication::Bearer(token) => request.bearer(token),
        }
    }

    /// Create a resource from a local file with a single conditional PUT. File transfers
    /// use the explicit byte bound and idle timeout so progressing large uploads can finish.
    /// The source must remain immutable until the call returns.
    ///
    /// # Errors
    /// Returns a policy, I/O, size, transport or HTTP error, including 412 for an occupied name.
    pub async fn create_file(
        &self,
        path: &ResourcePath,
        file: &std::path::Path,
        limit: u64,
    ) -> Result<(), Error> {
        self.create_file_with_progress(path, file, limit, |_| {})
            .await
    }

    /// Conditional file PUT with cumulative sent bytes delivered on the polling task.
    /// Providers without intermediate upload events report completion after the PUT succeeds.
    /// # Errors
    /// Returns the same failures as [`Self::create_file`].
    pub async fn create_file_with_progress(
        &self,
        path: &ResourcePath,
        file: &std::path::Path,
        limit: u64,
        mut progress: impl FnMut(u64),
    ) -> Result<(), Error> {
        use std::future::{Future, poll_fn};
        self.admit()?;
        let size = file.metadata().map_err(Error::LocalIo)?.len();
        if size > limit {
            return Err(Error::Limit);
        }
        let sent = Arc::new((AtomicU64::new(0), atomic_waker::AtomicWaker::new()));
        let events = sent.clone();
        let request = self
            .request(Method::Put, path)
            .header("If-None-Match", "*")
            .body_file(file)
            .upload_progress(move |bytes, _| {
                if events.0.fetch_max(bytes, Ordering::Relaxed) < bytes {
                    events.1.wake();
                }
            });
        let mut exchange = std::pin::pin!(
            self.inner
                .http
                .fetch_limited_future(request, self.inner.policy.max_response_bytes)
        );
        let mut reported = 0;
        progress(0);
        let reply = poll_fn(|cx| {
            sent.1.register(cx.waker());
            let result = exchange.as_mut().poll(cx);
            let latest = sent.0.load(Ordering::Relaxed).min(size);
            if latest > reported {
                reported = latest;
                progress(latest);
            }
            result
        })
        .await
        .map_err(|source| Error::Transport {
            source,
            outcome: WriteOutcome::MayHaveApplied,
        })?;
        self.mutation_status(&reply, path, &[200, 201, 204])?;
        if reported < size {
            progress(size);
        }
        Ok(())
    }

    /// Stream a complete GET into a staging sink with a hard bound on received bytes.
    ///
    /// # Errors
    /// Returns a policy, HTTP, size, sink, or transport error. Discard incomplete output.
    pub async fn read_file(
        &self,
        path: &ResourcePath,
        sink: &mut dyn std::io::Write,
        limit: u64,
    ) -> Result<(), Error> {
        self.admit()?;
        let failure = |source| Error::Transport {
            source,
            outcome: WriteOutcome::NotSent,
        };
        let stream = self
            .inner
            .http
            .send_future(self.request(Method::Get, path))
            .await
            .map_err(failure)?;
        if stream.status() != 200 {
            let head = Response {
                status: stream.status(),
                headers: stream.headers().to_vec(),
                body: Vec::new(),
                url: String::new(),
                metrics: None,
            };
            return self.status(&head, &[200], false);
        }
        let expected = stream
            .header("Content-Length")
            .map(str::parse::<u64>)
            .transpose()
            .map_err(|_| Error::Protocol)?;
        if expected.is_some_and(|n| n > limit) {
            return Err(Error::Limit);
        }
        let mut received = 0u64;
        let mut body = stream.into_body();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(failure)?;
            received = received
                .checked_add(chunk.len() as u64)
                .filter(|n| *n <= limit)
                .ok_or(Error::Limit)?;
            sink.write_all(&chunk).map_err(Error::LocalIo)?;
        }
        if expected.is_some_and(|n| n != received) {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    async fn exchange(
        &self,
        method: Method,
        path: &ResourcePath,
        bytes: Vec<u8>,
        headers: Vec<(String, String)>,
        safe: bool,
    ) -> Result<Response, Error> {
        let p = &self.inner.policy;
        if bytes.len() > p.max_upload_bytes {
            return Err(Error::Limit);
        }
        let started = self.inner.clock.now();
        let mut attempt = 0u8;
        let salt = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        loop {
            self.admit()?;
            attempt = attempt.saturating_add(1);
            let remaining = p
                .retry
                .max_elapsed
                .saturating_sub(self.inner.clock.now().saturating_sub(started));
            if safe && remaining.is_zero() {
                return Err(Error::Transport {
                    source: http::HttpError::Timeout,
                    outcome: WriteOutcome::NotSent,
                });
            }
            let mut request = self.request(method, path).timeout_total(if safe {
                p.request_timeout.min(remaining)
            } else {
                p.request_timeout
            });
            if !bytes.is_empty() {
                request = request.body(bytes.clone());
            }
            for (name, value) in &headers {
                request = request.header(name, value);
            }
            let response = self
                .inner
                .http
                .fetch_limited_future(request, p.max_response_bytes)
                .await;
            let (transient, after) = match &response {
                Ok(r) => (
                    matches!(r.status, 408 | 423 | 425 | 429 | 500 | 502 | 503 | 504),
                    crate::policy::retry_after(r.header("retry-after"), crate::policy::wall_now(r)),
                ),
                Err(e) => (
                    matches!(
                        e,
                        http::HttpError::Timeout
                            | http::HttpError::Dns
                            | http::HttpError::Connect
                            | http::HttpError::Io(_)
                    ),
                    None,
                ),
            };
            if safe
                && transient
                && let Some(delay) = p.retry.delay(attempt, after, salt)
            {
                let elapsed = self.inner.clock.now().saturating_sub(started);
                if elapsed.saturating_add(delay) < p.retry.max_elapsed {
                    let (tx, rx) = day_async::oneshot();
                    self.inner
                        .clock
                        .schedule(delay, Box::new(move || tx.send(())));
                    let _ = rx.await;
                    continue;
                }
            }
            return response.map_err(|source| Error::Transport {
                source,
                outcome: if safe {
                    WriteOutcome::NotSent
                } else {
                    WriteOutcome::MayHaveApplied
                },
            });
        }
    }
}
fn condition_headers(condition: Condition) -> Result<Vec<(String, String)>, Error> {
    Ok(match condition {
        Condition::Unconditional => vec![],
        Condition::CreateOnly => vec![("If-None-Match".into(), "*".into())],
        Condition::IfMatch(tag) => {
            if tag.is_weak() {
                return Err(Error::InvalidEntityTag);
            }
            vec![("If-Match".into(), tag.0)]
        }
    })
}
