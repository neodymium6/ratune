use ratune_subsonic::SubsonicClient;
use std::collections::HashMap;
mod support;
use support::fixture;

#[tokio::test]
async fn jukebox_connect_only_reads_and_parses_server_queue() {
    let (client, server) = fixture(
        "200 OK",
        r#"{"subsonic-response":{"status":"ok","jukeboxPlaylist":{"currentIndex":0,"playing":true,"gain":0.35,"position":17,"entry":[{"id":"remote","title":"Remote"}]}}}"#,
    );
    let response = client
        .jukebox_control(&ratune_subsonic::JukeboxCommand::Get)
        .await
        .unwrap();
    let ratune_subsonic::JukeboxResponse::Playlist(p) = response else {
        panic!()
    };
    assert_eq!(p.entry[0].id, "remote");
    assert!(p.status.playing);
    assert_eq!(p.status.position, 17.0);
    let request = server.join().unwrap();
    assert!(request.contains("/rest/jukeboxControl?"));
    assert!(request.contains("action=get"));
    assert!(!request.contains("test-password"));
    assert!(!request.contains("action=start"));
}

#[tokio::test]
async fn jukebox_set_preserves_repeated_ids_and_auth() {
    let (client, server) = fixture(
        "200 OK",
        r#"{"subsonic-response":{"status":"ok","jukeboxStatus":{"currentIndex":0,"playing":false,"gain":0.2,"position":0}}}"#,
    );
    client
        .jukebox_control(&ratune_subsonic::JukeboxCommand::Set(vec![
            "id one".into(),
            "id/two".into(),
            "id one".into(),
        ]))
        .await
        .unwrap();
    let request = server.join().unwrap();
    let path = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
    let ids: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "id")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(ids, ["id one", "id/two", "id one"]);
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["action"], "set");
    assert_eq!(q["u"], "test-user");
    assert_eq!(
        q["t"],
        format!("{:x}", md5::compute(format!("test-password{}", q["s"])))
    );
}

#[tokio::test]
async fn jukebox_rejects_errors_missing_payloads_and_redacts_http_urls() {
    for (status, body) in [
        ("200 OK", r#"{"subsonic-response":{"status":"ok"}}"#),
        (
            "200 OK",
            r#"{"subsonic-response":{"status":"failed","error":{"code":50,"message":"Not authorized"}}}"#,
        ),
        ("503 Unavailable", "unavailable"),
        ("200 OK", "invalid JSON"),
    ] {
        let (client, server) = fixture(status, body);
        let error = format!(
            "{:#}",
            client
                .jukebox_control(&ratune_subsonic::JukeboxCommand::Get)
                .await
                .unwrap_err()
        );
        for private in ["test-password", "test-user", "http://", "t="] {
            assert!(!error.contains(private));
        }
        server.join().unwrap();
    }
}

#[tokio::test]
async fn jukebox_invalid_commands_fail_without_network_requests() {
    let client = SubsonicClient::new("http://127.0.0.1:1", "fixture", "fixture").unwrap();
    for command in [
        ratune_subsonic::JukeboxCommand::Gain(f64::NAN),
        ratune_subsonic::JukeboxCommand::Gain(1.1),
        ratune_subsonic::JukeboxCommand::Set(vec![]),
    ] {
        let error = client
            .jukebox_control(&command)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sending request"));
    }
}

#[tokio::test]
async fn jukebox_permission_errors_remain_typed() {
    let (client, server) = fixture(
        "200 OK",
        r#"{"subsonic-response":{"status":"failed","error":{"code":50,"message":"Jukebox disabled"}}}"#,
    );
    let error = client
        .jukebox_control(&ratune_subsonic::JukeboxCommand::Get)
        .await
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error
            .downcast_ref::<ratune_subsonic::SubsonicError>()
            .unwrap()
            .code,
        50
    );
}
