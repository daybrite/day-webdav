// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Opt-in, synthetic large-object qualification using `DAV_LIVE_URL`, `DAV_LIVE_USERNAME`, and `DAV_LIVE_PASSWORD`.
//! Each provider creates a unique folder and removes it, including after a failed check.
use day_webdav::{http, *};
use futures_lite::future::block_on;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[test]
#[ignore = "requires explicit authorization for a live WebDAV account"]
fn native_large_objects() {
    exercise(http::Provider::Native);
}
#[cfg(all(
    feature = "reqwest",
    not(target_arch = "wasm32"),
    not(target_env = "ohos")
))]
#[test]
#[ignore = "requires explicit authorization for a live WebDAV account"]
fn reqwest_large_objects() {
    exercise(http::Provider::Reqwest);
}

fn exercise(provider: http::Provider) {
    let endpoint = Endpoint::new(&std::env::var("DAV_LIVE_URL").unwrap()).unwrap();
    let auth = Authentication::Basic {
        username: std::env::var("DAV_LIVE_USERNAME").unwrap(),
        password: std::env::var("DAV_LIVE_PASSWORD").unwrap(),
    };
    let session = http::Session::new();
    session.set_provider(provider);
    let client = Client::builder(endpoint)
        .session(session)
        .authentication(auth)
        .build();
    let folder = format!(
        "day-webdav-qualification-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = ResourcePath::from_segments([&folder], true).unwrap();
    block_on(client.mkcol(&root)).unwrap();
    println!("synthetic fixture: {folder}");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        block_on(async {
            let mut failures = Vec::new();
            // Include empty bodies and both the day-sync part size and default DAV upper bound.
            for size in [0, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024, 8 * 1024 * 1024] {
                let name = format!("synthetic-{size}.bin");
                let file = ResourcePath::from_segments([folder.as_str(), &name], false).unwrap();
                let bytes: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
                let start = Instant::now();
                let write = client
                    .put(&file, bytes.clone(), Condition::CreateOnly)
                    .await;
                println!(
                    "PUT {size} bytes in {:?}: {:?}",
                    start.elapsed(),
                    write.as_ref().map(|r| (r.status, &r.metrics))
                );
                if let Err(e) = write {
                    failures.push(format!("PUT {size}: {e:?}"));
                }
                // Also reconcile ambiguous writes; do not blindly replay a mutation.
                let start = Instant::now();
                match client.get(&file, None).await {
                    Ok(response) if response.body == bytes => {
                        println!("GET {size} bytes verified in {:?}", start.elapsed());
                    }
                    Ok(response) => failures.push(format!(
                        "GET {size}: wrong payload ({} bytes)",
                        response.body.len()
                    )),
                    Err(e) => failures.push(format!("GET {size}: {e:?}")),
                }
            }
            assert!(failures.is_empty(), "{}", failures.join("\n"));
        });
    }));
    let cleanup = block_on(client.delete(&root, Condition::Unconditional));
    println!("cleanup {folder}: {:?}", cleanup.as_ref().map(|r| r.status));
    cleanup.unwrap();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
