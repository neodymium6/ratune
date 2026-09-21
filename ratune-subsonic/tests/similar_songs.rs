mod support;
use ratune_subsonic::SubsonicClient;
use std::collections::HashMap;
use std::net::TcpListener;
use support::fixture;

#[tokio::test]
async fn instant_mix_sends_song_id_and_all_required_auth_parameters() {
    let (client, server) = fixture(
        "200 OK",
        r#"{"subsonic-response":{"status":"ok","similarSongs2":{"song":[{"id":"b","title":"B","coverArt":"art-b","duration":60}]}}}"#,
    );
    let songs = client
        .get_similar_songs2("song/id & unicode 日本語", 50)
        .await
        .unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].id, "b");
    assert_eq!(songs[0].cover_art.as_deref(), Some("art-b"));
    assert_eq!(songs[0].duration, Some(60));
    let request = server.join().unwrap();
    let path = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
    assert_eq!(url.path(), "/rest/getSimilarSongs2");
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["id"], "song/id & unicode 日本語");
    assert_eq!(q["count"], "50");
    assert_eq!(q["u"], "test-user");
    assert_eq!(q["v"], "1.16.1");
    assert_eq!(q["c"], "ratune");
    assert_eq!(q["f"], "json");
    assert_eq!(
        q["t"],
        format!("{:x}", md5::compute(format!("test-password{}", q["s"])))
    );
    assert!(!q.contains_key("p"));
    assert!(!request.contains("test-password"));
}

#[tokio::test]
async fn instant_mix_accepts_empty_song_list_or_empty_container() {
    for body in [
        r#"{"subsonic-response":{"status":"ok","similarSongs2":{"song":[]}}}"#,
        r#"{"subsonic-response":{"status":"ok","similarSongs2":{}}}"#,
    ] {
        let (client, server) = fixture("200 OK", body);
        assert!(client
            .get_similar_songs2("seed", 50)
            .await
            .unwrap()
            .is_empty());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn instant_mix_rejects_missing_payload_and_api_errors() {
    for (body, expected) in [
        (
            r#"{"subsonic-response":{"status":"ok"}}"#,
            "missing 'similarSongs2'",
        ),
        (
            r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#,
            "40",
        ),
        (
            r#"{"subsonic-response":{"status":"failed","error":{"code":0,"message":"Not implemented"}}}"#,
            "Not implemented",
        ),
    ] {
        let (client, server) = fixture("200 OK", body);
        let error = client.get_similar_songs2("seed", 50).await.unwrap_err();
        assert!(error.to_string().contains(expected));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn instant_mix_http_and_decode_errors_do_not_expose_authenticated_urls() {
    for (status, body) in [("503 Unavailable", "unavailable"), ("200 OK", "not json")] {
        let (client, server) = fixture(status, body);
        let error = client.get_similar_songs2("seed", 50).await.unwrap_err();
        let message = format!("{error:#}");
        assert!(!message.contains("test-user"));
        assert!(!message.contains("test-password"));
        assert!(!message.contains("http://"));
        assert!(!message.contains("t="));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn instant_mix_connection_errors_do_not_expose_authenticated_urls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = SubsonicClient::new(&base, "private-user", "private-password").unwrap();
    let message = format!(
        "{:#}",
        client.get_similar_songs2("seed", 50).await.unwrap_err()
    );
    assert!(!message.contains("private-"));
    assert!(!message.contains("http://"));
    assert!(!message.contains("t="));
}
