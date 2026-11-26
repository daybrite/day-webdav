// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#![cfg(all(feature = "demo-server", not(target_arch = "wasm32")))]
use day_webdav::*;

#[test]
fn native_demo_fixture() {
    exercise(http::Provider::Native);
}

#[cfg(all(feature = "reqwest", not(target_env = "ohos")))]
#[test]
fn reqwest_demo_fixture() {
    exercise(http::Provider::Reqwest);
}

fn exercise(provider: http::Provider) {
    let server = testing::Server::start().unwrap();
    let session = http::Session::new();
    session.set_provider(provider);
    let client = Client::builder(Endpoint::loopback(&server.url()).unwrap())
        .session(session)
        .build();
    futures_lite::future::block_on(async {
        assert!(client.options().await.unwrap().dav.contains(&"1".into()));
        let file = ResourcePath::from_segments(["café + 100%.bin"], false).unwrap();
        let body = b"fixture bytes\x00\xff".to_vec();
        client
            .put(&file, body.clone(), Condition::CreateOnly)
            .await
            .unwrap();
        assert_eq!(client.get(&file, None).await.unwrap().body, body);
        let response = client.head(&file).await.unwrap();
        let tag = EntityTag::parse(response.header("etag").unwrap()).unwrap();
        assert_eq!(client.get(&file, Some(&tag)).await.unwrap().status, 304);
        std::assert_matches!(client.get_range(&file, 1, 4, &tag).await.unwrap(),
            RangeDownload::Partial { response, .. } if response.body == b"ixtu");
        std::assert_matches!(
            client.put(&file, vec![], Condition::CreateOnly).await,
            Err(Error::Status { status: 412, .. })
        );
        assert_eq!(
            client
                .propfind(&ResourcePath::root(), Depth::One)
                .await
                .unwrap()
                .resources
                .len(),
            3
        );
        let mut source = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut source, &body).unwrap();
        let streaming = ResourcePath::from_segments(["streamed.bin"], false).unwrap();
        let size = body.len() as u64;
        std::assert_matches!(
            client
                .create_file(&streaming, source.path(), size - 1)
                .await,
            Err(Error::Limit)
        );
        let mut observations = Vec::new();
        let caller = std::thread::current().id();
        client
            .create_file_with_progress(&streaming, source.path(), size, |n| {
                assert_eq!(caller, std::thread::current().id());
                observations.push(n);
            })
            .await
            .unwrap();
        assert_eq!(observations.first(), Some(&0));
        assert_eq!(observations.last(), Some(&size));
        std::assert_matches!(
            client.create_file(&streaming, source.path(), size).await,
            Err(Error::Status { status: 412, .. })
        );
        let mut received = Vec::new();
        std::assert_matches!(
            client.read_file(&streaming, &mut received, size - 1).await,
            Err(Error::Limit)
        );
        assert_eq!(received, b"");
        client
            .read_file(&streaming, &mut received, size)
            .await
            .unwrap();
        assert_eq!(received, body);
        client.delete(&file, Condition::IfMatch(tag)).await.unwrap();
        std::assert_matches!(
            client.get(&file, None).await,
            Err(Error::Status { status: 404, .. })
        );
        let interrupted = ResourcePath::from_segments(["__drop__"], false).unwrap();
        assert!(
            client
                .read_file(&interrupted, &mut std::io::sink(), 1024)
                .await
                .is_err()
        );
        std::assert_matches!(
            client.get(&interrupted, None).await,
            Err(Error::Transport { .. }),
            "a truncated Content-Length must never return successful partial bytes"
        );
    });
}

#[test]
fn new_account_creates_only_the_app_folder_and_cleans_up_its_probe() {
    let server = testing::Server::start().unwrap();
    let endpoint = Endpoint::loopback(&format!("{}books/", server.url())).unwrap();
    let client = Client::builder(endpoint).build();
    futures_lite::future::block_on(async {
        let missing = client.check_app_folder(false).await.unwrap_err();
        assert_eq!(missing.step, ConnectionStep::OpenCollection);
        std::assert_matches!(missing.source, Error::Status { status: 404, .. });
        client.check_app_folder(true).await.unwrap();
        client.check_app_folder(false).await.unwrap();
        assert_eq!(
            client
                .propfind(&ResourcePath::root(), Depth::One)
                .await
                .unwrap()
                .resources
                .len(),
            1
        );
    });
}
