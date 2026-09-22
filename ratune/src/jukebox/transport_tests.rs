use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

fn read_request(socket: &mut TcpStream) -> String {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|s| s == b"\r\n\r\n") {
        let n = socket.read(&mut buffer).unwrap();
        assert!(n > 0);
        request.extend_from_slice(&buffer[..n]);
    }
    String::from_utf8(request).unwrap()
}

fn fixture(
    responses: Vec<(&'static str, &'static str)>,
) -> (SubsonicClient, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = SubsonicClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture",
        "fixture",
    )
    .unwrap();
    let server = thread::spawn(move || {
        let mut actions = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing fixture request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                }
            };
            let request = read_request(&mut socket);
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let query = path.split_once('?').unwrap().1;
            let action = query
                .split('&')
                .find_map(|part| part.strip_prefix("action="))
                .unwrap();
            actions.push(action.to_string());
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        actions
    });
    (client, server)
}

const PLAYLIST: &str = r#"{"subsonic-response":{"status":"ok","jukeboxPlaylist":{"currentIndex":0,"playing":true,"gain":0.5,"position":12,"entry":[{"id":"one","title":"One"},{"id":"two","title":"Two"}]}}}"#;
const STATUS: &str = r#"{"subsonic-response":{"status":"ok","jukeboxStatus":{"currentIndex":0,"playing":true,"gain":0.5,"position":12}}}"#;

fn expected() -> JukeboxPlaylist {
    serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(PLAYLIST).unwrap()["subsonic-response"]
            ["jukeboxPlaylist"]
            .clone(),
    )
    .unwrap()
}

#[test]
fn fixture_waits_for_delayed_and_fragmented_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut socket, _) = listener.accept().unwrap();
    // Exercise inherited nonblocking mode on every platform.
    socket.set_nonblocking(true).unwrap();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || tx.send(read_request(&mut socket)).unwrap());

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    client.write_all(b"GET /?action=get HTTP/1.1\r\n").unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    client.write_all(b"Host: fixture\r\n\r\n").unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "GET /?action=get HTTP/1.1\r\nHost: fixture\r\n\r\n"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn remote_operation_is_preflighted_serialized_and_read_back() {
    let (client, server) = fixture(vec![
        ("200 OK", PLAYLIST),
        ("200 OK", STATUS),
        ("200 OK", STATUS),
        ("200 OK", PLAYLIST),
    ]);
    let result = execute(&client, &expected(), Intent::Play(1))
        .await
        .unwrap();
    assert_eq!(result.entry.len(), 2);
    assert_eq!(server.join().unwrap(), ["get", "skip", "start", "get"]);
}

#[tokio::test]
async fn external_queue_change_is_not_overwritten() {
    let (client, server) = fixture(vec![("200 OK", PLAYLIST)]);
    let mut old = expected();
    old.entry.pop();
    assert!(execute(&client, &old, Intent::Clear)
        .await
        .unwrap_err()
        .contains("changed elsewhere"));
    assert_eq!(server.join().unwrap(), ["get"]);
}

#[tokio::test]
async fn failed_mutation_is_not_retried_or_followed_by_playback() {
    let (client, server) = fixture(vec![("200 OK", PLAYLIST), ("503 Unavailable", "failed")]);
    let error = execute(&client, &expected(), Intent::Play(1))
        .await
        .unwrap_err();
    assert!(!error.contains("http"));
    assert_eq!(server.join().unwrap(), ["get", "skip"]);
}

#[tokio::test]
async fn invalid_remote_position_is_rejected() {
    let (client, server) = fixture(vec![(
        "200 OK",
        r#"{"subsonic-response":{"status":"ok","jukeboxPlaylist":{"currentIndex":-1,"playing":false,"gain":0.5,"position":-2,"entry":[]}}}"#,
    )]);
    assert!(read(&client)
        .await
        .unwrap_err()
        .contains("Invalid Jukebox status"));
    assert_eq!(server.join().unwrap(), ["get"]);
}
