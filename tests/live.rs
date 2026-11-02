// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Every operation against a real WebDAV account, such as Fastmail Files
//! (`DAV_LIVE_URL=https://myfiles.fastmail.com/` with an app password that has Files access):
//!
//! ```sh
//! DAV_LIVE_URL=… DAV_LIVE_USERNAME=… DAV_LIVE_PASSWORD=… \
//!   cargo test --features reqwest --test live -- --ignored --nocapture
//! ```
//!
//! Each run makes a new folder, `DAV_LIVE_FOLDER` or `day-webdav live <UTC time>`, with one
//! subfolder per HTTP provider, and leaves it behind so the files can be inspected in the
//! provider's own interface. Every step is reported, and a failed step does not stop the ones
//! after it; the test fails at the end if any did.
use day_webdav::*;
use futures_lite::future::block_on;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[ignore = "needs a real account: DAV_LIVE_URL, DAV_LIVE_USERNAME, DAV_LIVE_PASSWORD"]
fn live_account_operations() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let endpoint = Endpoint::new(&var("DAV_LIVE_URL")).expect("an https DAV_LIVE_URL");
    let auth = Authentication::Basic {
        username: var("DAV_LIVE_USERNAME"),
        password: var("DAV_LIVE_PASSWORD"),
    };
    let folder = std::env::var("DAV_LIVE_FOLDER")
        .unwrap_or_else(|_| format!("day-webdav live {}", utc_now()));
    let providers = [
        Some(("native", http::Provider::Native)),
        #[cfg(all(
            feature = "reqwest",
            not(target_arch = "wasm32"),
            not(target_env = "ohos")
        ))]
        Some(("reqwest", http::Provider::Reqwest)),
    ];
    let providers: Vec<_> = providers.into_iter().flatten().collect();

    let mut report = Report::default();
    block_on(async {
        let client = |provider| {
            let session = http::Session::new();
            session.set_provider(provider);
            Client::builder(endpoint.clone())
                .session(session)
                .authentication(auth.clone())
                .build()
        };
        let first = client(providers[0].1.clone());
        let root = path([folder.as_str()], true);
        let made = report
            .step("create the run folder (MKCOL)", async {
                first.mkcol(&root).await?;
                Ok(folder.clone())
            })
            .await;
        if !made {
            return;
        }
        report
            .step("upload the run's README.md (PUT)", async {
                let text = format!(
                    "# {folder}\n\nWritten by day-webdav's live test, one subfolder per HTTP \
                 provider ({}). Each holds what the run left behind on purpose:\n\n\
                 - `notes.txt`, replaced once under `If-Match`\n\
                 - `café + 100%.txt`, a name that needs percent-encoding\n\
                 - `data.json` and `day.svg`, to preview\n\
                 - `pattern.bin`, 64 KiB fetched back in a byte range\n\
                 - `renamed.txt`, a COPY that was then MOVEd\n\
                 - `nested/deep.txt`, inside a nested collection\n\n\
                 Delete the folder when you are done with it.\n",
                    providers.iter().map(|p| p.0).collect::<Vec<_>>().join(", ")
                );
                first
                    .put(
                        &path([folder.as_str(), "README.md"], false),
                        text.into_bytes(),
                        Condition::CreateOnly,
                    )
                    .await?;
                Ok(String::new())
            })
            .await;
        for (name, provider) in &providers {
            report.section(name);
            exercise(&mut report, &client(provider.clone()), &folder, name).await;
        }
    });
    report.finish(&folder);
}

async fn exercise(report: &mut Report, client: &Client, folder: &str, name: &str) {
    let at = |file: &str| path([folder, name, file], false);
    let dir = path([folder, name], true);
    let notes = at("notes.txt");
    let pattern: Vec<u8> = (0..65_536u32).map(|i| (i * 31 % 251) as u8).collect();
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "notes.txt",
            format!("Version 1 of the {name} provider's notes.\n").into_bytes(),
        ),
        (
            "café + 100%.txt",
            "A name with an accent, a plus, a space and a percent sign.\n".into(),
        ),
        (
            "data.json",
            format!("{{\"provider\": \"{name}\", \"bytes\": [0, 255], \"ok\": true}}\n")
                .into_bytes(),
        ),
        ("day.svg", SVG.replace("{name}", name).into_bytes()),
        ("pattern.bin", pattern.clone()),
    ];

    report
        .step("discover the server (OPTIONS)", async {
            let d = client.options().await?;
            check(d.dav.iter().any(|c| c == "1"), "DAV class 1")?;
            Ok(format!("DAV {:?}; allows {:?}", d.dav, d.allow))
        })
        .await;
    report
        .step("list the account root (PROPFIND depth 0)", async {
            let listing = client.propfind(&ResourcePath::root(), Depth::Zero).await?;
            check(
                listing
                    .resources
                    .first()
                    .is_some_and(Resource::is_collection),
                "a collection",
            )?;
            Ok(format!("{} resource", listing.resources.len()))
        })
        .await;
    if !report
        .step("create the provider folder (MKCOL)", async {
            client.mkcol(&dir).await?;
            Ok(String::new())
        })
        .await
    {
        return;
    }
    report
        .step("refuse to create it twice (MKCOL → 405)", async {
            expect_status(client.mkcol(&dir).await, &[405])
        })
        .await;
    for (file, bytes) in &files {
        report
            .step(&format!("upload {file} (PUT, create-only)"), async {
                let r = client
                    .put(&at(file), bytes.clone(), Condition::CreateOnly)
                    .await?;
                Ok(format!("{} bytes, status {}", bytes.len(), r.status))
            })
            .await;
    }
    report
        .step("create a nested collection and a file in it", async {
            client.mkcol(&path([folder, name, "nested"], true)).await?;
            client
                .put(
                    &path([folder, name, "nested", "deep.txt"], false),
                    b"Two collections down.\n".to_vec(),
                    Condition::CreateOnly,
                )
                .await?;
            Ok(String::new())
        })
        .await;
    report
        .step("refuse to overwrite on create-only (PUT → 412)", async {
            expect_status(
                client
                    .put(&notes, b"clobbered".to_vec(), Condition::CreateOnly)
                    .await,
                &[412],
            )
        })
        .await;
    report
        .step("list the provider folder (PROPFIND depth 1)", async {
            let listing = client.propfind(&dir, Depth::One).await?;
            check(
                !listing.has_failures() || listing.resources.len() > 1,
                "a listing",
            )?;
            for (file, bytes) in &files {
                let encoded = at(file);
                let entry = listing
                    .resources
                    .iter()
                    .find(|r| {
                        r.href
                            .ends_with(encoded.as_encoded_str().rsplit('/').next().unwrap_or(""))
                    })
                    .ok_or(format!("{file} missing"))?;
                check(
                    entry.content_length() == Some(bytes.len() as u64),
                    &format!("{file} length"),
                )?;
            }
            Ok(format!("{} entries", listing.resources.len()))
        })
        .await;
    for (file, bytes) in &files {
        report
            .step(&format!("download {file} (GET) and compare"), async {
                let body = client.get(&at(file), None).await?.body;
                check(&body == bytes, "the uploaded bytes")?;
                Ok(String::new())
            })
            .await;
    }
    let mut tag = None;
    report
        .step("read the entity tag (HEAD)", async {
            let header = client
                .head(&notes)
                .await?
                .header("etag")
                .map(str::to_string);
            let parsed = EntityTag::parse(&header.ok_or("no ETag header")?)?;
            let text = parsed.as_str().to_string();
            tag = Some(parsed);
            Ok(text)
        })
        .await;
    report
        .step(
            "revalidate an unchanged file (GET If-None-Match → 304)",
            async {
                let t = tag.as_ref().ok_or("no entity tag")?;
                let status = client.get(&notes, Some(t)).await?.status;
                check(status == 304, &format!("304, got {status}"))?;
                Ok(String::new())
            },
        )
        .await;
    report
        .step("fetch bytes 1000–1999 (GET Range, If-Range)", async {
            let file = at("pattern.bin");
            let etag = client
                .head(&file)
                .await?
                .header("etag")
                .map(str::to_string)
                .ok_or("no ETag")?;
            let t = EntityTag::parse(&etag)?;
            if t.is_weak() {
                return Err(format!("weak ETag {etag}: ranges need a strong one").into());
            }
            match client.get_range(&file, 1000, 1999, &t).await? {
                RangeDownload::Partial {
                    response, total, ..
                } => {
                    check(response.body == pattern[1000..2000], "the right slice")?;
                    Ok(format!("206 of {total} bytes"))
                }
                RangeDownload::Restart(r) => {
                    Err(format!("server ignored Range ({})", r.status).into())
                }
            }
        })
        .await;
    report
        .step("never append across a stale validator (If-Range)", async {
            let stale = EntityTag::parse("\"day-webdav-stale\"")?;
            // RFC 9110 answers a stale If-Range with the whole file (200). A server that ignores
            // If-Range, as Fastmail's does, sends 206 under a different ETag, which the client
            // refuses as a protocol error instead: either way no slice of another version escapes.
            match client.get_range(&at("pattern.bin"), 0, 9, &stale).await {
                Ok(RangeDownload::Restart(r)) => Ok(format!("200, {} full bytes", r.body.len())),
                Err(Error::Protocol) => Ok("server ignored If-Range; its 206 was refused".into()),
                Ok(RangeDownload::Partial { .. }) => {
                    Err("appended across a changed validator".into())
                }
                Err(other) => Err(other.into()),
            }
        })
        .await;
    report
        .step("replace under the current tag (PUT If-Match)", async {
            let t = tag.clone().ok_or("no entity tag")?;
            let body =
                format!("Version 2 of the {name} provider's notes, written under If-Match.\n");
            client
                .put(&notes, body.clone().into_bytes(), Condition::IfMatch(t))
                .await?;
            check(
                client.get(&notes, None).await?.body == body.as_bytes(),
                "version 2",
            )?;
            Ok(String::new())
        })
        .await;
    report
        .step("refuse a stale tag (PUT If-Match → 412)", async {
            let t = tag.clone().ok_or("no entity tag")?;
            expect_status(
                client
                    .put(&notes, b"lost update".to_vec(), Condition::IfMatch(t))
                    .await,
                &[412],
            )
        })
        .await;
    let copy = at("notes copy.txt");
    report
        .step("copy a file (COPY)", async {
            let r = client.copy(&notes, &copy, false).await?;
            Ok(format!("status {}", r.status))
        })
        .await;
    report
        .step("refuse to copy over it (COPY Overwrite: F → 412)", async {
            expect_status(client.copy(&notes, &copy, false).await, &[412])
        })
        .await;
    report
        .step("copy over it when asked (COPY Overwrite: T)", async {
            let r = client.copy(&notes, &copy, true).await?;
            Ok(format!("status {}", r.status))
        })
        .await;
    report
        .step("rename the copy (MOVE)", async {
            client
                .move_resource(&copy, &at("renamed.txt"), false)
                .await?;
            expect_status(client.get(&copy, None).await, &[404])
        })
        .await;
    let scratch = at("scratch.txt");
    report
        .step("delete a file (DELETE If-Match)", async {
            client
                .put(&scratch, b"short-lived".to_vec(), Condition::CreateOnly)
                .await?;
            let etag = client
                .head(&scratch)
                .await?
                .header("etag")
                .map(str::to_string)
                .ok_or("no ETag")?;
            client
                .delete(&scratch, Condition::IfMatch(EntityTag::parse(&etag)?))
                .await?;
            expect_status(client.get(&scratch, None).await, &[404])
        })
        .await;
    report
        .step("delete a collection (DELETE)", async {
            let gone = path([folder, name, "scratch dir"], true);
            client.mkcol(&gone).await?;
            client
                .put(
                    &path([folder, name, "scratch dir", "inner.txt"], false),
                    b"x".to_vec(),
                    Condition::CreateOnly,
                )
                .await?;
            client.delete(&gone, Condition::Unconditional).await?;
            expect_status(
                client.propfind(&gone, Depth::Zero).await.map(|_| ()),
                &[404],
            )
        })
        .await;
}

type Outcome = Result<String, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Report {
    failed: Vec<String>,
    passed: usize,
}
impl Report {
    fn section(&self, name: &str) {
        println!("\n── {name} provider");
    }
    async fn step(&mut self, name: &str, run: impl std::future::Future<Output = Outcome>) -> bool {
        match run.await {
            Ok(detail) => {
                self.passed += 1;
                println!(
                    "  ok    {name}{}",
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(" — {detail}")
                    }
                );
                true
            }
            Err(error) => {
                println!("  FAIL  {name} — {error:?}");
                self.failed.push(name.to_string());
                false
            }
        }
    }
    fn finish(self, folder: &str) {
        println!(
            "\n{} passed, {} failed; left behind in “{folder}”",
            self.passed,
            self.failed.len()
        );
        assert!(self.failed.is_empty(), "failed: {:#?}", self.failed);
    }
}

fn path<const N: usize>(segments: [&str; N], collection: bool) -> ResourcePath {
    ResourcePath::from_segments(segments, collection).expect("a valid test path")
}

fn check(ok: bool, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    if ok {
        Ok(())
    } else {
        Err(format!("expected {what}").into())
    }
}

/// A request the server must refuse with one of `statuses`.
fn expect_status<T>(result: Result<T, Error>, statuses: &[u16]) -> Outcome {
    match result {
        Err(Error::Status { status, .. }) if statuses.contains(&status) => Ok(format!("{status}")),
        Err(other) => Err(format!("expected {statuses:?}, got {other:?}").into()),
        Ok(_) => Err(format!("expected {statuses:?}, but it succeeded").into()),
    }
}

/// `YYYY-MM-DD HH.MM.SS` in UTC, without a date crate.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}.{:02}.{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120" viewBox="0 0 320 120">
  <rect width="320" height="120" rx="16" fill="#f6b73c"/>
  <circle cx="60" cy="60" r="28" fill="#fff6dc"/>
  <text x="104" y="54" font-family="sans-serif" font-size="22" fill="#3a2a00">day-webdav</text>
  <text x="104" y="82" font-family="sans-serif" font-size="16" fill="#3a2a00">uploaded by the {name} provider</text>
</svg>
"##;
