"""In-process, loopback-only fixture. No real music, credentials or outbound I/O."""

import copy
import io
import json
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


def song(song_id, title, track):
    return dict(id=song_id, title=title, artist="Fixture Artist", artistId="artist",
                album="Fixture Album", albumId="album", duration=120, track=track,
                suffix="wav", contentType="audio/wav")


class Fixture:
    def __init__(self, gallery=False, discovery_failures=False):
        self.lock = threading.RLock()
        self.discovery_failures = discovery_failures
        self.songs = [song(key, title, i + 1) for i, (key, title) in enumerate([
            ("seed", "Seed"), ("empty", "Empty"), ("error", "Error"),
            ("slow", "Slow"), ("mix-a", "Similar A"), ("mix-b", "Similar B"),
        ])]
        self.album = dict(id="album", name="Fixture Album", artist="Fixture Artist",
                          artistId="artist", songCount=len(self.songs), song=self.songs)
        self.albums = {"album": self.album}
        self.artist = dict(id="artist", name="Fixture Artist", albumCount=1,
                           album=[self.album])
        if gallery:
            for i in range(1, 19):
                album_id = f"gallery-{i:02}"
                tracks = [dict(self.songs[0], id=f"{album_id}-track-{n}",
                               title=f"Gallery track {n}", albumId=album_id, track=n)
                          for n in range(1, 4)]
                self.albums[album_id] = dict(self.album, id=album_id,
                                            name=f"Gallery {i:02} 日本語のアルバム",
                                            year=2000+i, songCount=3, song=tracks)
            self.artist.update(albumCount=len(self.albums), album=list(self.albums.values()))
        self.streams = 0
        self.requests = []
        audio = io.BytesIO()
        with wave.open(audio, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(8000)
            wav.writeframes(bytes(120 * 8000 * 2))
        self.audio = audio.getvalue()

    def response(self, endpoint, params):
        seed = params.get("id", [""])[0]
        if endpoint == "getAlbumList2":
            albums = list(self.albums.values())
            if params.get("type") == ["newest"]:
                albums.reverse()
            return {"albumList2": {"album": copy.deepcopy(albums[:int(params.get("size", ["24"])[0])])}}
        if endpoint == "getAlbum" and self.discovery_failures:
            if seed == "gallery-18":
                return {"status": "failed", "error": {"code": 0, "message": "Fixture album unavailable"}}
            if seed == "gallery-17":
                return {"album": dict(self.albums[seed], song=[])}
            if seed == "gallery-16":
                time.sleep(5)
        if endpoint == "getSimilarSongs2":
            if params.get("count") != ["50"]:
                return {"status": "failed", "error": {"code": 10, "message": "Expected count=50"}}
            if seed == "slow":
                time.sleep(5)
            if seed == "error":
                return {"status": "failed", "error": {"code": 0, "message": "Fixture mix failure"}}
            similar = [] if seed == "empty" else [self.songs[4], self.songs[5], self.songs[4]]
            return {"similarSongs2": {"song": copy.deepcopy(similar)}}
        payloads = {
            "ping": {},
            "getArtists": {"artists": {"index": [{"name": "F", "artist": [self.artist]}]}},
            "getArtist": {"artist": self.artist},
            "getAlbum": {"album": self.albums.get(seed, self.album)},
            "getStarred2": {"starred2": {}},
            "getPlaylists": {"playlists": {}},
            "getInternetRadioStations": {"internetRadioStations": {}},
            "getScanStatus": {"scanStatus": {"scanning": False, "count": len(self.songs)}},
            "search3": {"searchResult3": {"song": self.songs if params.get("songOffset", ["0"])[0] == "0" else []}},
            "getSong": {"song": next((s for s in self.songs if s["id"] == seed), self.songs[0])},
        }
        return copy.deepcopy(payloads.get(endpoint, {
            "status": "failed", "error": {"code": 70, "message": "Fixture endpoint unavailable"},
        }))

    def __enter__(self):
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass  # Never print authentication query strings.

            def do_GET(self):
                url = urlsplit(self.path)
                endpoint = url.path.rsplit("/", 1)[-1].removesuffix(".view")
                params = parse_qs(url.query)
                with fixture.lock:
                    fixture.requests.append(endpoint)
                if endpoint == "stream":
                    with fixture.lock:
                        fixture.streams += 1
                    body, mime = fixture.audio, "audio/wav"
                else:
                    response = dict(status="ok", version="1.16.1", type="mock", serverVersion="fixture")
                    response.update(fixture.response(endpoint, params))
                    body = json.dumps({"subsonic-response": response}).encode()
                    mime = "application/json"
                try:
                    self.send_response(200)
                    self.send_header("Content-Type", mime)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
