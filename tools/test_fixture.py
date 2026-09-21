import io
import json
from pathlib import Path
import tempfile
import unittest
from urllib.request import urlopen
import wave

from mock_subsonic import Fixture
from tui_harness import prepare_state


class FixtureTests(unittest.TestCase):
    def test_similar_song_responses(self):
        fixture = Fixture()
        for seed, expected in [("seed", ["mix-a", "mix-b", "mix-a"]), ("empty", [])]:
            response = fixture.response("getSimilarSongs2", {"id": [seed], "count": ["50"]})
            self.assertEqual([song["id"] for song in response["similarSongs2"]["song"]], expected)
        response = fixture.response("getSimilarSongs2", {"id": ["error"], "count": ["50"]})
        self.assertEqual(response["status"], "failed")

    def test_loopback_api_and_silent_audio(self):
        with Fixture() as fixture:
            self.assertEqual(fixture.server.server_address[0], "127.0.0.1")
            with urlopen(fixture.url + "/rest/getArtists?u=fixture") as response:
                result = json.load(response)["subsonic-response"]
            self.assertEqual(result["artists"]["index"][0]["artist"][0]["id"], "artist")
            with urlopen(fixture.url + "/rest/stream?id=seed") as response:
                with wave.open(io.BytesIO(response.read())) as audio:
                    self.assertEqual(audio.getnframes(), 120 * 8000)
                    self.assertFalse(any(audio.readframes(audio.getnframes())))
            self.assertEqual(fixture.streams, 1)

    def test_state_is_fresh_synthetic_and_quiet(self):
        with Fixture() as fixture, tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            prepare_state(root, fixture)
            config = (root / "config/ratune/config.toml").read_text()
            self.assertIn(fixture.url, config)
            self.assertIn('password = "fixture"', config)
            state = json.loads((root / "config/ratune/state.json").read_text())
            self.assertEqual(state["player_volume"], 0)
            self.assertEqual(state["browser_focus"], "Artists")
            self.assertEqual(len(state["queue"]), 3)
            records = json.loads((root / "history.json").read_text())["records"]
            self.assertEqual(records[-1]["song_id"], "seed")
        with Fixture() as fresh:
            self.assertEqual(fresh.requests, [])
            self.assertEqual(fresh.streams, 0)


if __name__ == "__main__":
    unittest.main()
