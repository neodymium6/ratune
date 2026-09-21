"""Stateful server-side playback fixture; never produces audio."""

import copy


class JukeboxFixture:
    def __init__(self, fixture):
        self.fixture = fixture
        self.enabled = True
        self.fail_after_action = None
        self.state = dict(currentIndex=0, playing=True, gain=0.35, position=17,
                          entry=[dict(fixture.songs[0], id=key, title=title)
                                 for key, title in [("remote-one", "Remote One"),
                                                    ("remote-two", "Remote Two")]], commands=[])

    def snapshot(self):
        with self.fixture.lock:
            return dict(copy.deepcopy(self.state), streams=self.fixture.streams)

    def change(self, enabled=None):
        with self.fixture.lock:
            if enabled is None:
                self.state["entry"].append(dict(self.fixture.songs[0], id="external", title="External change"))
            else:
                self.enabled = enabled

    def response(self, params):
        with self.fixture.lock:
            if not self.enabled:
                return self.error("Jukebox is disabled")
            action = params.get("action", [""])[0]
            if action != "get":
                self.state["commands"].append(action)
            entries = self.state["entry"]
            if action in ("set", "add"):
                catalog = {s["id"]: s for a in self.fixture.albums.values() for s in a["song"]}
                catalog.update({s["id"]: s for s in entries})
                ids = params.get("id", [])
                if any(i not in catalog for i in ids):
                    return self.error("Track missing")
                songs = [copy.deepcopy(catalog[i]) for i in ids]
                self.state["entry"] = songs if action == "set" else entries + songs
                self.state["currentIndex"] = min(max(0, self.state["currentIndex"]), max(0, len(self.state["entry"])-1))
            elif action == "start":
                self.state["playing"] = bool(entries)
            elif action == "stop":
                self.state["playing"] = False
            elif action == "skip":
                index = int(params["index"][0])
                if not 0 <= index < len(entries):
                    return self.error("Invalid index")
                self.state.update(currentIndex=index, position=int(params.get("offset", ["0"])[0]))
            elif action == "setGain":
                self.state["gain"] = float(params["gain"][0])
            elif action == "clear":
                self.state.update(entry=[], currentIndex=-1, playing=False, position=0)
            elif action == "remove":
                entries.pop(int(params["index"][0]))
                self.state["currentIndex"] = min(self.state["currentIndex"], len(entries)-1)
            elif action == "shuffle":
                current = entries[self.state["currentIndex"]] if entries else None
                entries.reverse()
                self.state["currentIndex"] = entries.index(current) if current else -1
            elif action != "get":
                return self.error("Unsupported fixture action")
            if self.fail_after_action == action:
                self.fail_after_action = None
                return self.error("Response failed after applying mutation")
            status = {key: self.state[key] for key in ("currentIndex", "playing", "gain", "position")}
            if action == "get":
                return {"jukeboxPlaylist": dict(status, entry=copy.deepcopy(self.state["entry"]))}
            return {"jukeboxStatus": status}

    @staticmethod
    def error(message):
        return {"status": "failed", "error": {"code": 0, "message": message}}
