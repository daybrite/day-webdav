// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use crate::{Section, res};
use day::prelude::*;
use day_webdav::Resource as DavResource;
use day_webdav::{
    self as dav,
    http::{Provider, Session},
    *,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub(crate) struct Scene {
    pub section: Signal<Section>,
    pub endpoint: Signal<String>,
    pub username: Signal<String>,
    pub password: Signal<String>,
    pub auth: Signal<usize>,
    #[cfg(all(not(target_arch = "wasm32"), not(target_env = "ohos")))]
    pub provider: Signal<usize>,
    pub writes: Signal<bool>,
    pub constrained: Signal<bool>,
    pub expensive: Signal<bool>,
    pub path: Signal<String>,
    pub destination: Signal<String>,
    pub payload: Signal<String>,
    pub etag: Signal<String>,
    pub session: Signal<Session>,
    pub status: Signal<String>,
    pub busy: Signal<bool>,
    pub task: Signal<Option<day::TaskHandle>>,
    pub request: Signal<String>,
    pub response: Signal<String>,
    pub rows: Signal<Vec<DavResource>>,
    pub statistics: Signal<String>,
    pub files: Files,
}
impl Ambient for Scene {
    fn create() -> Self {
        Self {
            section: Signal::new(Section::Files),
            endpoint: Signal::new(fixture_url().unwrap_or_default()),
            username: Signal::new(String::new()),
            password: Signal::new(String::new()),
            auth: Signal::new(0),
            #[cfg(all(not(target_arch = "wasm32"), not(target_env = "ohos")))]
            provider: Signal::new(0),
            writes: Signal::new(true),
            constrained: Signal::new(false),
            expensive: Signal::new(false),
            path: Signal::new(String::new()),
            destination: Signal::new(String::new()),
            payload: Signal::new(res::str::fixture_payload().format()),
            etag: Signal::new(String::new()),
            session: Signal::new(Session::new()),
            status: Signal::new(res::str::status_ready().format()),
            busy: Signal::new(false),
            task: Signal::new(None),
            request: Signal::new(String::new()),
            response: Signal::new(String::new()),
            rows: Signal::new(vec![]),
            statistics: Signal::new(String::new()),
            files: Files::new(),
        }
    }
}
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::unnecessary_wraps,
        reason = "the native arm starts a fixture server, which can fail"
    )
)]
fn fixture_url() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static SERVER: std::sync::OnceLock<Option<dav::testing::Server>> =
            std::sync::OnceLock::new();
        SERVER
            .get_or_init(|| dav::testing::Server::start().ok())
            .as_ref()
            .map(day_webdav::testing::Server::url)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Some("http://127.0.0.1:18444/dav/".into())
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    Options,
    List,
    Get,
    Head,
    Mkcol,
    Put,
    Replace,
    Copy,
    Move,
    Delete,
    Smoke,
    Slow,
    Broken,
}
impl Operation {
    fn writes(self) -> bool {
        matches!(
            self,
            Self::Mkcol
                | Self::Put
                | Self::Replace
                | Self::Copy
                | Self::Move
                | Self::Delete
                | Self::Smoke
        )
    }
}
pub fn relative(input: &str, collection: bool) -> Result<ResourcePath, Error> {
    let input = input.trim_matches('/');
    if input.is_empty() {
        Ok(ResourcePath::root())
    } else {
        ResourcePath::from_segments(input.split('/'), collection)
    }
}
impl Scene {
    pub fn fixture(self) {
        self.cancel();
        if let Some(url) = fixture_url() {
            self.endpoint.set(url);
            self.username.set(String::new());
            self.password.set(String::new());
            self.auth.set(0);
            self.path.set(String::new());
            self.writes.set(true);
            self.status.set(res::str::status_ready().format());
        } else {
            self.status.set(res::str::fixture_failed().format());
        }
    }
    pub fn cancel(self) {
        if let Some(task) = self.task.get_untracked() {
            task.abort();
        }
        self.task.set(None);
        self.busy.set(false);
        self.status.set(res::str::status_cancelled().format());
    }
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the list's selection callback hands over an owned id"
    )]
    pub fn selected(self, id: String) {
        if let Some(resource) = self.rows.get_untracked().iter().find(|r| r.href == id) {
            if let Some(relative) = resource.href.strip_prefix(&self.endpoint.get_untracked()) {
                self.path.set(
                    percent_encoding::percent_decode_str(relative)
                        .decode_utf8_lossy()
                        .into_owned(),
                );
            }
            self.etag.set(resource.etag().unwrap_or_default().into());
            self.response.set(format!("{resource:#?}"));
        }
    }
    fn client(self) -> Result<Client, Error> {
        let authentication = match self.auth.get_untracked() {
            1 => Authentication::Basic {
                username: self.username.get_untracked(),
                password: self.password.get_untracked(),
            },
            2 => Authentication::Bearer(self.password.get_untracked()),
            _ => Authentication::None,
        };
        self.client_for(authentication, 1024 * 1024)
    }
    /// A client for the Connection page's endpoint, provider and network policy.
    fn client_for(self, authentication: Authentication, limit: usize) -> Result<Client, Error> {
        let raw = self.endpoint.get_untracked();
        let endpoint = Endpoint::new(&raw).or_else(|_| Endpoint::loopback(&raw))?;
        // Store the normalized base so resource selection uses the same confinement boundary.
        self.endpoint.set(endpoint.as_str().into());
        let session = self.session.get_untracked();
        #[cfg(all(not(target_arch = "wasm32"), not(target_env = "ohos")))]
        session.set_provider(if self.provider.get_untracked() == 1 {
            Provider::Reqwest
        } else {
            Provider::Native
        });
        #[cfg(any(target_arch = "wasm32", target_env = "ohos"))]
        session.set_provider(Provider::Native);
        let policy = Policy {
            max_response_bytes: limit,
            max_upload_bytes: limit,
            allow_expensive: self.expensive.get_untracked(),
            ..Default::default()
        };
        let client = Client::builder(endpoint)
            .session(session)
            .authentication(authentication)
            .policy(policy)
            .build();
        if self.constrained.get_untracked() {
            client.set_network_state(NetworkState::Constrained);
        }
        Ok(client)
    }
    pub fn run(self, op: Operation) {
        self.cancel();
        if op.writes() && !self.writes.get_untracked() {
            self.status.set(res::str::write_disabled().format());
            return;
        }
        let Ok(client) = self.client() else {
            self.status.set(res::str::endpoint_invalid().format());
            return;
        };
        self.busy.set(true);
        self.status.set(res::str::status_working().format());
        self.response.set(String::new());
        let input = self.path.get_untracked();
        let dest = self.destination.get_untracked();
        let body = self.payload.get_untracked().into_bytes();
        let etag = self.etag.get_untracked();
        // Protocol operation/URL are inspection data, not app-owned labels. Credentials excluded.
        self.request
            .set(format!("{op:?}\n{}\n{input}", client.endpoint().as_str()));
        let handle = day::task(async move {
            let result = execute(&client, op, &input, &dest, body, &etag).await;
            self.busy.set(false);
            match result {
                Ok(output) => {
                    self.status.set(if matches!(op, Operation::Smoke) {
                        res::str::smoke_ok().format()
                    } else {
                        res::str::status_ok().format()
                    });
                    self.response.set(output.text);
                    if let Some(rows) = output.rows {
                        self.rows.set(rows);
                    }
                }
                Err(e) => {
                    self.status.set(if matches!(e, Error::Deferred(_)) {
                        res::str::status_deferred().format()
                    } else {
                        res::str::status_error(e.to_string()).format()
                    });
                    self.response.set(format!("{e:#?}"));
                }
            }
            let stats = self.session.get_untracked().statistics();
            self.statistics.set(
                res::str::transfer_summary(res::str::TransferSummary {
                    count: stats.started.to_string(),
                    uploaded: stats.uploaded_bytes.to_string(),
                    downloaded: stats.downloaded_bytes.to_string(),
                })
                .format(),
            );
        });
        self.task.set(Some(handle));
        Scope::current().on_cleanup(move || handle.abort());
    }
}
struct Output {
    text: String,
    rows: Option<Vec<DavResource>>,
}
async fn execute(
    client: &Client,
    op: Operation,
    path: &str,
    destination: &str,
    body: Vec<u8>,
    etag: &str,
) -> Result<Output, Error> {
    let path = relative(path, matches!(op, Operation::List | Operation::Mkcol))?;
    let text = match op {
        Operation::Smoke => return smoke(client).await,
        Operation::Options => format!("{:#?}", client.options().await?),
        Operation::List => {
            let result = client.propfind(&path, Depth::One).await?;
            return Ok(Output {
                text: format!("{result:#?}"),
                rows: Some(result.resources),
            });
        }
        _ => {
            let response = match op {
                Operation::Get => client.get(&path, None).await?,
                Operation::Head => client.head(&path).await?,
                Operation::Mkcol => client.mkcol(&path).await?,
                Operation::Put => client.put(&path, body, Condition::CreateOnly).await?,
                Operation::Replace => {
                    client
                        .put(&path, body, Condition::IfMatch(EntityTag::parse(etag)?))
                        .await?
                }
                Operation::Copy => {
                    client
                        .copy(&path, &relative(destination, false)?, false)
                        .await?
                }
                Operation::Move => {
                    client
                        .move_resource(&path, &relative(destination, false)?, false)
                        .await?
                }
                Operation::Delete => {
                    if path.as_encoded_str().is_empty() {
                        return Err(Error::InvalidPath);
                    }
                    client.delete(&path, Condition::Unconditional).await?
                }
                Operation::Slow => client.get(&relative("__slow__", false)?, None).await?,
                Operation::Broken => client.get(&relative("__drop__", false)?, None).await?,
                _ => unreachable!(),
            };
            let headers = response
                .headers
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.to_ascii_lowercase().as_str(),
                        "set-cookie" | "authorization" | "proxy-authenticate"
                    )
                })
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "HTTP {}\n{headers}\n\n{}",
                response.status,
                String::from_utf8_lossy(&response.body[..response.body.len().min(16384)])
            )
        }
    };
    Ok(Output { text, rows: None })
}
async fn smoke(client: &Client) -> Result<Output, Error> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "day-fixture-{}-{}",
        dav::http::simulation::Clock::now(&dav::http::simulation::RealClock).as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let dir = ResourcePath::from_segments([suffix.as_str()], true)?;
    let file = ResourcePath::from_segments([suffix.as_str(), "café + 100%.bin"], false)?;
    let copy = ResourcePath::from_segments([suffix.as_str(), "copy.bin"], false)?;
    let moved = ResourcePath::from_segments([suffix.as_str(), "moved.bin"], false)?;
    client.options().await?;
    client.mkcol(&dir).await?;
    // Cleanup only our successfully created collection, including when a check fails.
    let result=async {
        let bytes=b"Synthetic protocol fixture\x00\xff".to_vec();client.put(&file,bytes.clone(),Condition::CreateOnly).await?;
        let listed=client.propfind(&dir,Depth::One).await?;if listed.resources.len()!=2{return Err(Error::Protocol);}
        let response=client.get(&file,None).await?;if response.body!=bytes{return Err(Error::Protocol);}
        let tag=EntityTag::parse(response.header("etag").ok_or(Error::Protocol)?)?;
        if client.get(&file,Some(&tag)).await?.status!=304{return Err(Error::Protocol);}
        client.head(&file).await?;
        if !matches!(client.put(&file,vec![0],Condition::CreateOnly).await,Err(Error::Status {status:412,..})){return Err(Error::Protocol);}
        if tag.is_weak(){return Err(Error::InvalidEntityTag);}
        if !matches!(client.get_range(&file,1,4,&tag).await?,RangeDownload::Partial {response,..} if response.body==b"ynth"){return Err(Error::Protocol);}
        client.copy(&file,&copy,false).await?;client.move_resource(&copy,&moved,false).await?;
        if client.get(&moved,None).await?.body!=bytes{return Err(Error::Protocol);}
        client.delete(&moved,Condition::Unconditional).await?;
        if !matches!(client.get(&moved,None).await,Err(Error::Status {status:404,..})){return Err(Error::Protocol);}
        Ok(Output {text:format!("OPTIONS → MKCOL → PUT → PROPFIND → GET → HEAD → GET 304 → PUT 412 → RANGE → COPY → MOVE → DELETE → GET 404\n{listed:#?}"),rows:Some(vec![])})
    }.await;
    let cleanup = client.delete(&dir, Condition::Unconditional).await;
    match (result, cleanup) {
        (Ok(output), Ok(_)) => Ok(output),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

/// The largest file the Files page previews or replaces in one exchange.
const PREVIEW_LIMIT: usize = 8 * 1024 * 1024;
/// Fastmail's `WebDAV` root, which signs in with an app password that has Files access.
pub const FASTMAIL: &str = "https://myfiles.fastmail.com/";

/// One listed resource. `key` is the decoded path relative to the account root; a collection's
/// ends in `/`, so a file and a folder of the same name never share a key.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub parent: Option<String>,
    pub name: String,
    pub collection: bool,
    pub size: Option<u64>,
    pub etag: Option<String>,
    pub modified: Option<String>,
}

/// What the preview pane shows for the selected entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Nothing,
    Loading,
    Folder,
    Text,
    Image,
    Binary,
    Failed(String),
}

/// The Files page: one signed-in account, its lazily listed tree, and the selected entry's
/// preview. Every async result checks `generation` first, so a reply that lands after a sign-out
/// (or a sign-in to another account) is dropped rather than merged into the wrong tree.
#[derive(Clone, Copy)]
pub(crate) struct Files {
    pub client: Signal<Option<Client>>,
    pub generation: Signal<u64>,
    pub account: Signal<String>,
    pub nodes: Signal<Vec<Entry>>,
    pub expanded: Signal<HashSet<String>>,
    pub loaded: Signal<HashSet<String>>,
    pub selection: Signal<Vec<String>>,
    pub preview: Signal<Preview>,
    pub text: Signal<String>,
    pub original: Signal<String>,
    pub is_text: Signal<bool>,
    pub image: Signal<Option<Arc<Vec<u8>>>>,
    pub etag: Signal<Option<String>>,
    pub content_type: Signal<String>,
    pub status: Signal<String>,
    pub busy: Signal<bool>,
    pub preview_task: Signal<Option<day::TaskHandle>>,
}

impl Files {
    pub fn new() -> Self {
        Self {
            client: Signal::new(None),
            generation: Signal::new(0),
            account: Signal::new(String::new()),
            nodes: Signal::new(vec![]),
            expanded: Signal::new(HashSet::new()),
            loaded: Signal::new(HashSet::new()),
            selection: Signal::new(vec![]),
            preview: Signal::new(Preview::Nothing),
            text: Signal::new(String::new()),
            original: Signal::new(String::new()),
            is_text: Signal::new(false),
            image: Signal::new(None),
            etag: Signal::new(None),
            content_type: Signal::new(String::new()),
            status: Signal::new(String::new()),
            busy: Signal::new(false),
            preview_task: Signal::new(None),
        }
    }
    pub fn signed_in(self) -> bool {
        self.client.with(Option::is_some)
    }
    /// The selected entry's key, if any.
    pub fn current(self) -> Option<String> {
        self.selection.with_untracked(|s| s.first().cloned())
    }
    pub fn entry(self, key: &str) -> Option<Entry> {
        self.nodes
            .with_untracked(|n| n.iter().find(|e| e.key == key).cloned())
    }
    /// The selected entry, tracked: the preview pane's header follows renames and saves.
    pub fn selected_entry(self) -> Option<Entry> {
        let key = self.selection.with(|s| s.first().cloned())?;
        self.nodes
            .with(|n| n.iter().find(|e| e.key == key).cloned())
    }
    pub fn dirty(self) -> bool {
        self.is_text.get() && self.text.with(|t| self.original.with(|o| t != o))
    }
    /// Where New folder and New file put their item: the selected folder, the selected file's
    /// folder, or the account root.
    fn target_folder(self) -> Option<String> {
        let key = self.current()?;
        if key.ends_with('/') {
            Some(key)
        } else {
            parent_of(&key)
        }
    }
    fn live(self, generation: u64) -> Option<Client> {
        if self.generation.get_untracked() == generation {
            self.client.get_untracked()
        } else {
            None
        }
    }
    fn clear_preview(self) {
        if let Some(task) = self.preview_task.get_untracked() {
            task.abort();
        }
        self.preview_task.set(None);
        self.preview.set(Preview::Nothing);
        self.text.set(String::new());
        self.original.set(String::new());
        self.is_text.set(false);
        self.image.set(None);
        self.etag.set(None);
        self.content_type.set(String::new());
    }
    pub fn sign_out(self) {
        self.generation.update(|g| *g += 1);
        self.clear_preview();
        self.client.set(None);
        self.account.set(String::new());
        self.nodes.set(vec![]);
        self.expanded.set(HashSet::new());
        self.loaded.set(HashSet::new());
        self.selection.set(vec![]);
        self.busy.set(false);
        self.status.set(String::new());
    }
    /// List `parent` again and merge its children into the tree.
    async fn refresh(self, generation: u64, parent: Option<String>) -> Result<(), Error> {
        let Some(client) = self.live(generation) else {
            return Ok(());
        };
        let key = parent.clone().unwrap_or_default();
        let listing = client.propfind(&relative(&key, true)?, Depth::One).await?;
        if self.live(generation).is_none() {
            return Ok(());
        }
        let children = entries(
            client.endpoint().as_str(),
            parent.as_deref(),
            &listing.resources,
        );
        self.nodes.update(|n| merge(n, parent.as_deref(), children));
        let keys = self
            .nodes
            .with_untracked(|n| n.iter().map(|e| e.key.clone()).collect::<HashSet<_>>());
        // A folder that left the listing takes its loaded and expanded state with it.
        let keep = |k: &String| k.is_empty() || keys.contains(k);
        self.loaded.update(|l| {
            l.insert(key);
            l.retain(keep);
        });
        self.expanded.update(|x| x.retain(keep));
        Ok(())
    }
    /// List a folder the first time it opens.
    pub fn expand(self, key: String, open: bool) {
        if !open || self.loaded.with_untracked(|l| l.contains(&key)) {
            return;
        }
        self.load(Some(key));
    }
    pub fn load(self, parent: Option<String>) {
        let generation = self.generation.get_untracked();
        day::task(async move {
            if let Err(e) = self.refresh(generation, parent).await {
                self.status.set(describe(&e, None));
            }
        });
    }
    pub fn reload(self) {
        let generation = self.generation.get_untracked();
        let mut folders: Vec<Option<String>> = vec![None];
        folders.extend(
            self.loaded
                .get_untracked()
                .into_iter()
                .filter(|k| !k.is_empty())
                .map(Some),
        );
        let selected = self.current();
        self.busy.set(true);
        day::task(async move {
            let mut failure = None;
            for folder in folders {
                if let Err(e) = self.refresh(generation, folder).await {
                    failure.get_or_insert(e);
                }
            }
            if self.live(generation).is_none() {
                return;
            }
            self.busy.set(false);
            if let Some(e) = failure {
                self.status.set(describe(&e, None));
            } else {
                self.status.set(res::str::files_refreshed().format());
                // Show the server's current version, or let go of an entry that is gone.
                self.select(selected.filter(|k| self.entry(k).is_some()));
            }
        });
    }
    pub fn select(self, key: Option<String>) {
        self.clear_preview();
        self.selection.set(key.iter().cloned().collect());
        let Some(key) = key else { return };
        let Some(entry) = self.entry(&key) else {
            return;
        };
        if entry.collection {
            self.preview.set(Preview::Folder);
            self.expand(key, true);
            return;
        }
        let generation = self.generation.get_untracked();
        let (Some(client), Ok(path)) = (self.live(generation), relative(&key, false)) else {
            return;
        };
        self.preview.set(Preview::Loading);
        let handle = day::task(async move {
            let result = client.get(&path, None).await;
            if self.live(generation).is_none() || self.current().as_deref() != Some(&key) {
                return;
            }
            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    self.preview.set(Preview::Failed(describe(&e, None)));
                    return;
                }
            };
            let etag = response.header("etag").map(str::to_owned);
            let content_type = response
                .header("content-type")
                .unwrap_or_default()
                .to_owned();
            let kind = classify(&key, &content_type, &response.body);
            let size = response.body.len() as u64;
            self.etag.set(etag.clone());
            self.content_type.set(content_type);
            if matches!(kind, Kind::Text | Kind::Svg) {
                let text = String::from_utf8_lossy(&response.body).into_owned();
                self.original.set(text.clone());
                self.text.set(text);
                self.is_text.set(true);
            }
            if matches!(kind, Kind::Image | Kind::Svg) {
                self.image.set(Some(Arc::new(response.body)));
            }
            self.preview.set(match kind {
                Kind::Text => Preview::Text,
                Kind::Image | Kind::Svg => Preview::Image,
                Kind::Binary => Preview::Binary,
            });
            self.nodes.update(|n| {
                if let Some(e) = n.iter_mut().find(|e| e.key == key) {
                    e.size = Some(size);
                    if etag.is_some() {
                        e.etag = etag;
                    }
                }
            });
        });
        self.preview_task.set(Some(handle));
    }
    /// The If-Match condition for the previewed file. Without a strong ETag the server cannot
    /// check for a concurrent change, so the write is unconditional.
    fn condition(self) -> Condition {
        self.etag
            .get_untracked()
            .and_then(|t| EntityTag::parse(&t).ok())
            .filter(|t| !t.is_weak())
            .map_or(Condition::Unconditional, Condition::IfMatch)
    }
    /// Write `body` over the selected file, conditional on the version the preview loaded.
    fn write(self, body: Vec<u8>, edited: Option<String>) {
        let generation = self.generation.get_untracked();
        let (Some(key), Some(client)) = (self.current(), self.live(generation)) else {
            return;
        };
        let Ok(path) = relative(&key, false) else {
            return;
        };
        let condition = self.condition();
        self.busy.set(true);
        self.status.set(res::str::files_saving().format());
        day::task(async move {
            let size = body.len() as u64;
            let result = async {
                let response = client.put(&path, body, condition).await?;
                Ok::<_, Error>(match response.header("etag") {
                    Some(tag) => Some(tag.to_owned()),
                    None => client.head(&path).await?.header("etag").map(str::to_owned),
                })
            }
            .await;
            if self.live(generation).is_none() {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(etag) => {
                    self.status.set(res::str::files_saved().format());
                    self.nodes.update(|n| {
                        if let Some(e) = n.iter_mut().find(|e| e.key == key) {
                            e.size = Some(size);
                            e.etag.clone_from(&etag);
                        }
                    });
                    match edited {
                        // The editor already shows what was written; it is now the saved text.
                        Some(text) if self.current().as_deref() == Some(&key) => {
                            self.original.set(text);
                            self.etag.set(etag);
                        }
                        Some(_) => {}
                        None => self.select(Some(key)),
                    }
                }
                Err(e) => self.status.set(describe(
                    &e,
                    Some(res::str::files_changed_remotely().format()),
                )),
            }
        });
    }
    pub fn save(self) {
        let text = self.text.get_untracked();
        self.write(text.clone().into_bytes(), Some(text));
    }
    pub fn revert(self) {
        self.text.set(self.original.get_untracked());
    }
    /// Replace the selected file's contents with a file chosen on this device.
    pub fn replace(self) {
        day::task(async move {
            let Some(file) = open_file()
                .title(res::str::files_replace_title())
                .present()
                .await
            else {
                return;
            };
            match file.read() {
                Ok(bytes) if bytes.len() <= PREVIEW_LIMIT => self.write(bytes, None),
                Ok(_) => self.status.set(res::str::files_too_large().format()),
                Err(_) => self.status.set(res::str::files_unreadable().format()),
            }
        });
    }
    pub fn rename(self) {
        let generation = self.generation.get_untracked();
        let Some(entry) = self.current().and_then(|k| self.entry(&k)) else {
            return;
        };
        day::task(async move {
            let Some(name) = prompt(res::str::files_rename_title())
                .initial(entry.name.clone())
                .ok_label(res::str::files_rename())
                .await
            else {
                return;
            };
            let name = name.trim().to_owned();
            if name == entry.name {
                return;
            }
            if !valid_name(&name) {
                self.status.set(res::str::files_bad_name().format());
                return;
            }
            let Some(client) = self.live(generation) else {
                return;
            };
            let parent = entry.parent.clone();
            let key = format!(
                "{}{name}{}",
                parent.clone().unwrap_or_default(),
                if entry.collection { "/" } else { "" }
            );
            self.busy.set(true);
            let result = async {
                let from = relative(&entry.key, entry.collection)?;
                client
                    .move_resource(&from, &relative(&key, entry.collection)?, false)
                    .await?;
                self.refresh(generation, parent).await
            }
            .await;
            if self.live(generation).is_none() {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(()) => {
                    self.status.set(res::str::files_renamed().format());
                    self.select(Some(key));
                }
                Err(e) => self
                    .status
                    .set(describe(&e, Some(res::str::files_name_taken().format()))),
            }
        });
    }
    pub fn delete(self) {
        let generation = self.generation.get_untracked();
        let Some(entry) = self.current().and_then(|k| self.entry(&k)) else {
            return;
        };
        day::task(async move {
            let body = if entry.collection {
                res::str::files_delete_folder_body()
            } else {
                res::str::files_delete_file_body()
            };
            let confirmed = confirm(res::str::files_delete_title(entry.name.clone()))
                .message(body)
                .confirm_label(res::str::files_delete())
                .destructive()
                .await;
            let Some(client) = self.live(generation).filter(|_| confirmed) else {
                return;
            };
            // A file is deleted only if it is still the version listed; a folder has no
            // version to compare.
            let condition = match entry.etag.as_deref().map(EntityTag::parse) {
                Some(Ok(tag)) if !entry.collection && !tag.is_weak() => Condition::IfMatch(tag),
                _ => Condition::Unconditional,
            };
            self.busy.set(true);
            let result = async {
                client
                    .delete(&relative(&entry.key, entry.collection)?, condition)
                    .await
            }
            .await;
            if self.live(generation).is_none() {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(_) => {
                    self.nodes.update(|n| remove(n, &entry.key));
                    if self.current().as_deref() == Some(&entry.key) {
                        self.select(None);
                    }
                    self.status.set(res::str::files_deleted().format());
                }
                Err(e) => self.status.set(describe(
                    &e,
                    Some(res::str::files_changed_remotely().format()),
                )),
            }
        });
    }
    /// Create a folder or an empty file in [`Files::target_folder`], then select it.
    pub fn create(self, collection: bool) {
        let generation = self.generation.get_untracked();
        let parent = self.target_folder();
        day::task(async move {
            let title = if collection {
                res::str::files_new_folder_title()
            } else {
                res::str::files_new_file_title()
            };
            let Some(name) = prompt(title).ok_label(res::str::files_create()).await else {
                return;
            };
            let name = name.trim().to_owned();
            if !valid_name(&name) {
                self.status.set(res::str::files_bad_name().format());
                return;
            }
            let Some(client) = self.live(generation) else {
                return;
            };
            let key = format!(
                "{}{name}{}",
                parent.clone().unwrap_or_default(),
                if collection { "/" } else { "" }
            );
            self.busy.set(true);
            let result = async {
                let path = relative(&key, collection)?;
                if collection {
                    client.mkcol(&path).await?;
                } else {
                    client.put(&path, vec![], Condition::CreateOnly).await?;
                }
                self.refresh(generation, parent.clone()).await
            }
            .await;
            if self.live(generation).is_none() {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(()) => {
                    if let Some(folder) = parent {
                        self.expanded.update(|x| {
                            x.insert(folder);
                        });
                    }
                    self.status.set(res::str::files_created().format());
                    self.select(Some(key));
                }
                Err(e) => self
                    .status
                    .set(describe(&e, Some(res::str::files_name_taken().format()))),
            }
        });
    }
}

impl Scene {
    /// Sign the Files page in with the Connection fields: Basic authentication when a username
    /// is given, none otherwise (the local fixture).
    pub fn sign_in(self) {
        let files = self.files;
        let username = self.username.get_untracked().trim().to_owned();
        let authentication = if username.is_empty() {
            self.auth.set(0);
            Authentication::None
        } else {
            self.auth.set(1);
            Authentication::Basic {
                username: username.clone(),
                password: self.password.get_untracked(),
            }
        };
        let Ok(client) = self.client_for(authentication, PREVIEW_LIMIT) else {
            files.status.set(res::str::endpoint_invalid().format());
            return;
        };
        files.sign_out();
        let generation = files.generation.get_untracked();
        files.busy.set(true);
        files.status.set(res::str::files_signing_in().format());
        day::task(async move {
            let result = client.propfind(&ResourcePath::root(), Depth::One).await;
            if files.generation.get_untracked() != generation {
                return;
            }
            files.busy.set(false);
            match result {
                Ok(listing) => {
                    let endpoint = client.endpoint().as_str().to_owned();
                    files
                        .nodes
                        .set(entries(&endpoint, None, &listing.resources));
                    files.loaded.set(HashSet::from([String::new()]));
                    let host = endpoint
                        .split("://")
                        .nth(1)
                        .and_then(|rest| rest.split('/').next())
                        .unwrap_or_default()
                        .to_owned();
                    files.account.set(if username.is_empty() {
                        host
                    } else {
                        res::str::files_account(res::str::FilesAccount {
                            user: username,
                            host,
                        })
                        .format()
                    });
                    files.client.set(Some(client));
                    files.status.set(String::new());
                }
                Err(e) => files.status.set(describe(&e, None)),
            }
        });
    }
    pub fn use_fastmail(self) {
        self.endpoint.set(FASTMAIL.into());
        self.auth.set(1);
    }
}

/// A message for a failed Files operation; `conflict` explains a 412 in the operation's terms.
fn describe(e: &Error, conflict: Option<String>) -> String {
    match e {
        Error::Status {
            status: 401 | 403, ..
        } => res::str::files_denied().format(),
        Error::Status { status: 404, .. } => res::str::files_missing().format(),
        Error::Status { status: 412, .. } => {
            conflict.unwrap_or_else(|| res::str::files_changed_remotely().format())
        }
        Error::Status { status: 507, .. } => res::str::files_full().format(),
        Error::Status { status, .. } => res::str::files_status(i64::from(*status)).format(),
        Error::Limit => res::str::files_too_large().format(),
        Error::Deferred(_) => res::str::status_deferred().format(),
        Error::Transport { .. } => res::str::files_offline().format(),
        _ => res::str::status_error(e.to_string()).format(),
    }
}

/// A name for one path segment: not empty, no separator, not a relative step.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

/// The folder key that holds `key`, or `None` at the account root.
pub fn parent_of(key: &str) -> Option<String> {
    key.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/"))
}

/// The children of `parent` in a Depth 1 listing, folders first, then by name.
pub fn entries(endpoint: &str, parent: Option<&str>, resources: &[DavResource]) -> Vec<Entry> {
    let own = parent.unwrap_or_default();
    let mut out: Vec<Entry> = resources
        .iter()
        .filter_map(|r| {
            let relative = r.href.strip_prefix(endpoint).or_else(|| {
                // The root's own response may omit the trailing slash.
                (r.href == endpoint.trim_end_matches('/')).then_some("")
            })?;
            let decoded = percent_encoding::percent_decode_str(relative)
                .decode_utf8_lossy()
                .trim_end_matches('/')
                .to_owned();
            let collection = r.is_collection();
            let key = if collection && !decoded.is_empty() {
                format!("{decoded}/")
            } else {
                decoded
            };
            if key == own || key.is_empty() {
                return None;
            }
            let name = key.trim_end_matches('/').rsplit('/').next()?.to_owned();
            Some(Entry {
                parent: parent.map(str::to_owned),
                name,
                collection,
                size: r.content_length(),
                etag: r.etag().map(str::to_owned),
                modified: r
                    .property("DAV:", "getlastmodified")
                    .map(|p| p.text.trim().to_owned()),
                key,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.collection
            .cmp(&a.collection)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Replace `parent`'s children with a fresh listing. A folder still listed keeps its loaded
/// descendants; one that is gone takes them with it.
pub fn merge(nodes: &mut Vec<Entry>, parent: Option<&str>, children: Vec<Entry>) {
    let gone: Vec<String> = nodes
        .iter()
        .filter(|e| e.parent.as_deref() == parent && !children.iter().any(|c| c.key == e.key))
        .map(|e| e.key.clone())
        .collect();
    nodes.retain(|e| {
        e.parent.as_deref() != parent
            && !gone
                .iter()
                .any(|g| g.ends_with('/') && e.key.starts_with(g.as_str()))
    });
    nodes.extend(children);
}

/// Drop `key` and, for a folder, everything listed under it.
pub fn remove(nodes: &mut Vec<Entry>, key: &str) {
    nodes.retain(|e| e.key != key && !(key.ends_with('/') && e.key.starts_with(key)));
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Text,
    Image,
    /// Drawn as an image, and editable as the text it is.
    Svg,
    Binary,
}

/// Decide how to preview a file from its name, its Content-Type, and a look at its bytes.
pub fn classify(key: &str, content_type: &str, body: &[u8]) -> Kind {
    const IMAGES: [&str; 9] = [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic",
    ];
    let extension = key
        .rsplit_once('.')
        .map_or_default(|(_, e)| e.to_ascii_lowercase());
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if extension == "svg" || mime == "image/svg+xml" {
        return Kind::Svg;
    }
    if mime.starts_with("image/") || IMAGES.contains(&extension.as_str()) {
        return Kind::Image;
    }
    let head = &body[..body.len().min(8192)];
    let texty = mime.starts_with("text/")
        || ["json", "xml", "javascript", "yaml", "toml"]
            .iter()
            .any(|t| mime.contains(t));
    // A cut at the 8 KiB mark can split a character; only the bytes before it must be valid.
    let utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    if utf8
        && !head.contains(&0)
        && (texty || mime.is_empty() || mime == "application/octet-stream")
    {
        Kind::Text
    } else {
        Kind::Binary
    }
}

/// A byte count for people: bytes, then KB and MB with one decimal.
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        res::str::size_bytes(bytes as i64).format()
    } else if bytes < 1024 * 1024 {
        res::str::size_kb(format!("{:.1}", b / 1024.0)).format()
    } else {
        res::str::size_mb(format!("{:.1}", b / (1024.0 * 1024.0))).format()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str) -> Entry {
        Entry {
            key: key.into(),
            parent: parent_of(key),
            name: key.trim_end_matches('/').rsplit('/').next().unwrap().into(),
            collection: key.ends_with('/'),
            size: None,
            etag: None,
            modified: None,
        }
    }
    fn keys(nodes: &[Entry]) -> Vec<&str> {
        nodes.iter().map(|e| e.key.as_str()).collect()
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parent_of("notes.txt"), None);
        assert_eq!(parent_of("Docs/"), None);
        assert_eq!(parent_of("Docs/a.txt").as_deref(), Some("Docs/"));
        assert_eq!(parent_of("Docs/Old/").as_deref(), Some("Docs/"));
        assert!(valid_name("café + 100%.txt"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(".."));
    }

    #[test]
    fn merge_keeps_listed_folders_and_drops_gone_ones() {
        let mut nodes = vec![
            entry("Docs/"),
            entry("Old/"),
            entry("a.txt"),
            entry("Docs/inner.txt"),
            entry("Old/inner.txt"),
        ];
        merge(&mut nodes, None, vec![entry("Docs/"), entry("b.txt")]);
        // Docs/ is still listed, so its loaded child stays; Old/ and a.txt are gone.
        assert_eq!(keys(&nodes), ["Docs/inner.txt", "Docs/", "b.txt"]);
        merge(&mut nodes, Some("Docs/"), vec![entry("Docs/x.txt")]);
        assert_eq!(keys(&nodes), ["Docs/", "b.txt", "Docs/x.txt"]);
        remove(&mut nodes, "Docs/");
        assert_eq!(keys(&nodes), ["b.txt"]);
    }

    #[test]
    fn classifies_by_type_name_and_bytes() {
        assert_eq!(classify("a.md", "", b"# Title"), Kind::Text);
        assert_eq!(classify("a", "application/json", b"{}"), Kind::Text);
        assert_eq!(classify("a.svg", "", b"<svg/>"), Kind::Svg);
        assert_eq!(
            classify("a.png", "", &[0x89, b'P', b'N', b'G', 0]),
            Kind::Image
        );
        assert_eq!(classify("a.bin", "", &[0, 1, 2, 255]), Kind::Binary);
        assert_eq!(classify("a.pdf", "application/pdf", b"%PDF"), Kind::Binary);
        // A multi-byte character cut at the sniffing window's edge is still text.
        let text = "é".repeat(5000);
        assert_eq!(classify("a", "text/plain", text.as_bytes()), Kind::Text);
    }

    #[test]
    fn listing_becomes_tree_entries() {
        let server = dav::testing::Server::start().unwrap();
        let client = Client::builder(Endpoint::loopback(&server.url()).unwrap()).build();
        let path = |p: &str, c| relative(p, c).unwrap();
        futures_lite::future::block_on(async {
            client.mkcol(&path("Docs", true)).await.unwrap();
            for file in ["Docs/a b.txt", "café + 100%.txt"] {
                client
                    .put(&path(file, false), b"x".to_vec(), Condition::CreateOnly)
                    .await
                    .unwrap();
            }
            let root = client
                .propfind(&ResourcePath::root(), Depth::One)
                .await
                .unwrap();
            let base = client.endpoint().as_str();
            let top = entries(base, None, &root.resources);
            assert_eq!(keys(&top), ["Docs/", "café + 100%.txt", "fixture.txt"]);
            assert!(top[0].collection && top[0].parent.is_none());
            assert_eq!(top[1].size, Some(1));
            let docs = client
                .propfind(&path("Docs/", true), Depth::One)
                .await
                .unwrap();
            let inner = entries(base, Some("Docs/"), &docs.resources);
            assert_eq!(keys(&inner), ["Docs/a b.txt"]);
            assert_eq!(inner[0].name, "a b.txt");
            assert_eq!(inner[0].parent.as_deref(), Some("Docs/"));
        });
    }
}
