"""Own temporary state + private tmux server; never attach to user sessions."""

import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import uuid


def prepare_state(root, fixture):
    config = root / "config/ratune"
    config.mkdir(parents=True)
    (config / "config.toml").write_text(f'''[server]
url = "{fixture.url}"
username = "fixture"
password = "fixture"
[player]
default_volume = 0
[ui]
album_art_backend = "kitty-apc"
[cache]
enabled = false
[library]
enabled = false
[lyrics]
enabled = false
[scrobble]
enabled = false
scrobble_to_server = false
[radio]
enabled = false
''')
    state = dict(active_tab="browser", browser_focus="Artists",
                 queue=[fixture.songs[i] for i in (0, 4, 5)], queue_cursor=0,
                 player_volume=0)
    (config / "state.json").write_text(json.dumps(state))
    now = int(time.time())
    records = [dict(song_id=s["id"], album_id="album", artist_id="artist",
                    artist_name=s["artist"], album_name=s["album"],
                    track_name=s["title"], played_at=now - (3 - i) * 60,
                    duration_secs=120)
               for i, s in enumerate([fixture.songs[5], fixture.songs[4], fixture.songs[0]])]
    # The current application's history path falls back to cwd when HOME is absent.
    (root / "history.json").write_text(json.dumps({"records": records}))


class Tui:
    def __init__(self, binary, fixture):
        self.binary = Path(binary).resolve()
        self.fixture = fixture
        self.socket = "ratune-test-" + uuid.uuid4().hex
        self.temp = None

    def tmux(self, *args, check=True):
        result = subprocess.run(["tmux", "-L", self.socket, *args],
                                text=True, capture_output=True, timeout=10, check=check)
        return result.stdout

    def __enter__(self):
        if not self.binary.is_file():
            raise RuntimeError("Build ratune before running the TUI smoke test")
        self.temp = tempfile.TemporaryDirectory(prefix="ratune-fixture-")
        self.root = Path(self.temp.name)
        try:
            prepare_state(self.root, self.fixture)
            command = ["env", "-u", "HOME", "-u", "SUBSONIC_URL", "-u", "SUBSONIC_USER",
                       "-u", "SUBSONIC_PASS", "-u", "TERMUSIC_SUBSONIC_URL",
                       "-u", "TERMUSIC_SUBSONIC_USER", "-u", "TERMUSIC_SUBSONIC_PASS"]
            command.extend(f"XDG_{kind}_HOME={self.root / name}" for kind, name in [
                ("CONFIG", "config"), ("CACHE", "cache"), ("DATA", "data"), ("STATE", "state"),
            ])
            command.append(str(self.binary))
            self.tmux("-f", "/dev/null", "new-session", "-d", "-s", "fixture",
                      "-x", "160", "-y", "48", "-c", str(self.root), shlex.join(command))
            self.expect("Fixture Artist", timeout=20)
            return self
        except BaseException:
            self.__exit__()
            raise

    def screen(self):
        return self.tmux("capture-pane", "-p", "-t", "fixture:0.0")

    def key(self, *keys):
        for key in keys:
            self.tmux("send-keys", "-t", "fixture:0.0", key)
            time.sleep(0.22)

    def expect(self, text, timeout=10):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            output = self.screen()
            if text in output:
                return output
            time.sleep(0.1)
        raise AssertionError(f"Missing {text!r}:\n{self.screen()}")

    def state(self):
        return json.loads((self.root / "config/ratune/state.json").read_text())

    def quit(self):
        self.key("q")
        until = time.monotonic() + 5
        while time.monotonic() < until:
            if not self.tmux("list-sessions", check=False):
                return
            time.sleep(0.1)
        raise AssertionError("Fixture player did not quit")

    def __exit__(self, *_):
        # Only our UUID-named server; never the user's tmux socket or panes.
        try:
            self.tmux("kill-server", check=False)
        finally:
            if self.temp:
                self.temp.cleanup()


def baseline(tui):
    tui.key("3")
    tui.expect("Queue (3)")
    tui.expect("Seed")
    assert tui.fixture.streams == 0, "Restoring a queue must not start playback"
    tui.key("2", "i")
    tui.expect("Navigation")
    tui.key("Escape", "3")
    tui.expect("Queue (3)")
    tui.quit()
    assert [s["id"] for s in tui.state()["queue"]] == ["seed", "mix-a", "mix-b"]
    assert tui.state()["player_volume"] == 0
