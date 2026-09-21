use std::collections::HashMap;
mod support;
use support::fixture;

#[tokio::test]
async fn discovery_requests_bounded_newest_and_random_lists_in_server_order() {
    for (newest, size, kind, limit) in [
        (true, 24, "newest", "24"),
        (false, 900, "random", "100"),
        (true, 0, "newest", "1"),
    ] {
        let (client, server) = fixture(
            "200 OK",
            r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"b","name":"B","coverArt":"cover-b"},{"id":"a","name":"A"}]}}}"#,
        );
        let albums = client.get_discovery_albums(newest, size).await.unwrap();
        assert_eq!(
            albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(albums[0].cover_art.as_deref(), Some("cover-b"));
        let request = server.join().unwrap();
        let path = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
        assert_eq!(url.path(), "/rest/getAlbumList2");
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(q["type"], kind);
        assert_eq!(q["size"], limit);
        assert_eq!(q["u"], "test-user");
        assert_eq!(
            q["t"],
            format!("{:x}", md5::compute(format!("test-password{}", q["s"])))
        );
        assert!(!request.contains("test-password"));
    }
}

#[tokio::test]
async fn discovery_accepts_empty_albums_but_rejects_missing_payload_and_api_errors() {
    for body in [
        r#"{"subsonic-response":{"status":"ok","albumList2":{}}}"#,
        r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[]}}}"#,
    ] {
        let (client, server) = fixture("200 OK", body);
        assert!(client
            .get_discovery_albums(true, 24)
            .await
            .unwrap()
            .is_empty());
        server.join().unwrap();
    }
    for body in [
        r#"{"subsonic-response":{"status":"ok"}}"#,
        r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong credentials"}}}"#,
    ] {
        let (client, server) = fixture("200 OK", body);
        assert!(client.get_discovery_albums(true, 24).await.is_err());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn discovery_http_and_decode_errors_do_not_expose_authenticated_urls() {
    for (status, body) in [("503 Unavailable", "unavailable"), ("200 OK", "not json")] {
        let (client, server) = fixture(status, body);
        let message = format!(
            "{:#}",
            client.get_discovery_albums(true, 24).await.unwrap_err()
        );
        for private in ["test-user", "test-password", "http://", "t="] {
            assert!(!message.contains(private));
        }
        server.join().unwrap();
    }
}
