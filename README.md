# day-webdav

MPL-2.0 WebDAV foundation over `day-part-http`, created with `day new part`.
The library is headless; **[WebDAV Explorer](demo/README.md)** is its conventional Day demo.

This is an early release of a transport layer, not a synchronization engine, and its API may still
change. It needs `day-part-http` 0.6.3 or later, the first release with the DAV methods.

## Scope

- OPTIONS, Depth 0/1 PROPFIND, GET/HEAD, MKCOL, PUT, DELETE, COPY and MOVE.
- Namespace-aware Multi-Status parsing preserving individual property failures and extensions.
- HTTPS endpoints, confined and encoded paths, opaque ETags, explicit conditional writes.
- Bounded responses/uploads/XML; cancellable async requests; bounded read retries with jitter,
  Retry-After, idle and overall deadlines; explicit constrained/expensive/offline admission.
- Validated range downloads: a 200 response means restart, never append.
- Native HTTP by default; optional `reqwest` uses the same Day HTTP session/provider abstraction.

Mutations are **never automatically replayed by this crate**. A lost response means the write may
have applied. The future persistence layer must durably record intent, read back and verify immutable
objects, resolve conflicts, and advance cursors transactionally. HTTP conditional headers do not
make arbitrary DAV servers transactional. See [research and limitations](docs/interop.html).

## Example

```rust,no_run
use day_webdav::{Client, Depth, Endpoint, ResourcePath};

async fn list() -> Result<(), day_webdav::Error> {
    let client = Client::builder(Endpoint::new("https://cloud.example/dav/private/")?).build();
    let listing = client.propfind(&ResourcePath::root(), Depth::One).await?;
    for resource in listing.resources {
        println!("{} {:?}", resource.href, resource.etag());
    }
    Ok(())
}
```

Supply `Authentication::Basic` with an app password, or `Authentication::Bearer`. Secrets remain in
memory; credential acquisition/storage belongs to the application. Use a dedicated `http::Session`
for provider switching or request interception. Enable `reqwest`, then call
`session.set_provider(http::Provider::Reqwest)` before building a client. Web and HarmonyOS support
only the native provider in the current HTTP layer.

## Shared account settings

Day applications can embed the [day-piece-webdav-settings crate](crates/day-piece-webdav-settings/).
Supply localized labels and an `AccountFields` value with the app’s default folder name.
The server field accepts a custom DAV base URL, including paths such as a Nextcloud user’s
DAV collection. The suggestions include [Fastmail](https://www.fastmail.help/hc/en-us/articles/1500000277882-Remote-file-access-WebDAV),
[Koofr](https://koofr.eu/help/koofr_with_webdav/how-do-i-connect-a-service-to-koofr-through-webdav/)
and both [pCloud regions](https://help.pcloud.com/article/connect-to-pcloud-using-webdav-and-rsync).
Users enter the folder as a relative path with ordinary spaces and Unicode. An empty folder
uses the base collection itself.

`account_form` submits the canonical collection URL, relative folder, username and password.
Save the folder alongside the endpoint so the form can restore both fields. On a new account
with a nonempty folder, `Client::check_app_folder(true)` creates the final folder if missing.
Its parent must exist. For credential updates or an empty folder, pass `false`. The trial
creates a disposable child, tests conditional writes, lists and reads back its file, and
removes the child before returning success. Save credentials only after the trial succeeds.

Use `ConnectionFailure.step` and the HTTP status or `Error::diagnostic_code()` to populate
the shared `failure_details` disclosure. The diagnostic code excludes credentials, URLs,
response bodies and platform error text. Fleet Reader and Stanza Redux format these details
using their app catalogs.

## Develop and validate

With this crate and the modified Day checkout as siblings:

```sh
day patch --local ../day --project .
day patch --local ../day --project demo
cargo fmt --all -- --check
cargo test
cargo test --features reqwest,demo-server
cargo clippy --all-features --all-targets -- -D warnings
cd demo
day launch -t macos-appkit --script dayscript/demo.yaml
```

The root has no toolkit features. The demo declares all eight standard toolkit targets. Its
walkthrough sends real loopback HTTP through native and reqwest providers, where supported. CI
uses the shared `daylib.yml` workflow for host tests, cross checks and every declared target's demo.
No platform is silently counted as having a reqwest test when that provider is unavailable.

`tests/fixtures/*.json` are sanitized actual Apache and Nextcloud exchanges. They replay entirely
offline through `day-part-http` interception. Fault tests inject truncation, stalls, TLS errors,
rate limits and policy changes using the same HTTP simulation and a deterministic clock.
`tests/loopback.rs` additionally tests both providers against the demo fixture with real sockets.

For capture reproduction and evidence, see [docs/interop.html](docs/interop.html).
The live `tests/native.rs` tests are explicitly ignored without the capture harness, which supplies
fresh isolated server roots. No production server credentials belong in these tests or fixtures.

`tests/live_uploads.rs` checks empty files and exact-byte round trips at 256 KiB, 1 MiB, 4 MiB,
and the default 8 MiB transfer bound. It creates and deletes a unique synthetic folder per
provider. With an explicitly authorized account, supply `DAV_LIVE_URL`, `DAV_LIVE_USERNAME`,
and `DAV_LIVE_PASSWORD` in the environment and run:

```sh
cargo test --features reqwest --test live_uploads -- --ignored --nocapture --test-threads=1
```

A failed PUT is read back for diagnosis, never automatically replayed. Passing a live run
establishes compatibility for that workload; it does not guarantee service availability,
quota, retention, or behavior under every network failure.

## Deliberate boundaries

No LOCK/UNLOCK, PROPPATCH or RFC 6578 REPORT client API yet (the HTTP layer recognizes these verbs).
No automatic redirects, infinite-depth traversal, cookie login, Digest/NTLM integration, Nextcloud
chunked upload, streaming-to-file, durable retry queue, background OS scheduling or conflict resolver.
Large files need a later streaming/chunked design: the current API bounds each buffered transfer.
Browser use additionally requires server CORS permission and exposed protocol headers.
