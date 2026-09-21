"""Server playback checks against an isolated, silent Jukebox fixture."""


def jukebox(tui):
    remote = tui.fixture.jukebox

    def state():
        return remote.snapshot()

    def ids():
        return [song["id"] for song in state()["entry"]]

    def select(title):
        tui.key("2", "Escape", "Escape", "Enter", "g", "g", "Enter", "/", *title, "Enter")
        tui.expect(title)

    def control(key):
        tui.key(key)
        tui.expect("Jukebox: server confirmed")

    tui.expect("[Local]")
    local = tui.state()
    before = state()
    assert before["playing"] and before["commands"] == []
    tui.key("F8")
    tui.expect("Jukebox connected")
    tui.expect("Remote One")
    assert state() == before, "Connecting must not change playback, queue, volume or local streams"

    control("p")
    assert not state()["playing"]
    control("Right")
    assert state()["position"] == 27 and not state()["playing"]
    control("+")
    assert state()["gain"] == 0.4
    control("p")
    assert state()["playing"]
    control("n")
    assert state()["currentIndex"] == 1
    control("N")
    assert state()["currentIndex"] == 0

    # A failed response may follow a successful mutation: read back, do not retry.
    with tui.fixture.lock:
        remote.fail_after_action = "setGain"
    commands = state()["commands"]
    tui.key("+")
    tui.expect("Jukebox unavailable")
    assert state()["gain"] == 0.45
    assert state()["commands"] == commands + ["setGain"]
    tui.expect("[Jukebox]")

    remote.change(enabled=False)
    tui.expect("[Jukebox (stale)]")
    tui.key("p")
    tui.expect("state is stale")
    assert state()["commands"] == commands + ["setGain"]
    remote.change(enabled=True)
    tui.expect("[Jukebox]")

    select("Seed")
    control("a")
    assert ids() == ["remote-one", "remote-two", "seed"]
    control("m")
    assert ids() == ["seed", "mix-a", "mix-b"]
    for title, message in [("Empty", "no similar songs"), ("Error", "Cannot create Instant Mix")]:
        select(title)
        old_ids, commands = ids(), state()["commands"]
        tui.key("m")
        tui.expect(message)
        assert ids() == old_ids and state()["commands"] == commands

    select("Slow")
    tui.key("m")
    tui.expect("waiting for server confirmation")
    tui.key("q")
    tui.expect("wait before quitting")
    tui.key("F8")
    tui.expect("wait before disconnecting")
    commands = state()["commands"]
    remote.change()
    tui.expect("queue changed elsewhere")
    assert ids()[-1] == "external" and state()["commands"] == commands

    tui.key("1", "g", "g", "Enter")
    tui.expect("Cannot load album")
    tui.key("l", "Enter")
    tui.expect("choose 1–200 tracks")
    tui.key("l", "l")
    control("Enter")
    assert ids() == [f"gallery-15-track-{i}" for i in range(1, 4)]
    tui.key("1")
    control("a")
    assert len(ids()) == 6
    tui.key("3", "g", "g", "j")
    control("Enter")
    assert state()["currentIndex"] == 1
    control("d")
    assert len(ids()) == 5

    unchanged = state()
    assert unchanged["streams"] == 0, "Remote mode must never stream audio locally"
    tui.key("F8")
    tui.expect("Local output restored")
    tui.expect(f"Queue ({len(local['queue'])})")
    assert state() == unchanged, "Disconnect must leave server playback unchanged"

    remote.change(enabled=False)
    tui.key("F8")
    tui.expect("Jukebox unavailable")
    tui.expect("[Local]")
    remote.change(enabled=True)
    tui.key("F8")
    tui.expect("Jukebox connected")
    unchanged = state()
    tui.quit()
    saved = tui.state()
    # Deserialization supplies optional metadata fields absent from the minimal fixture.
    # Check order and every supplied value, allowing those defaults to be serialized.
    assert len(saved["queue"]) == len(local["queue"])
    for restored, original in zip(saved["queue"], local["queue"]):
        assert all(restored.get(key) == value for key, value in original.items())
    assert saved["queue_cursor"] == local["queue_cursor"]
    assert saved["player_volume"] == local["player_volume"]
    assert state() == unchanged, "Quit must preserve local state and server playback"
