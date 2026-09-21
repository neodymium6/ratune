"""Instant Mix scenarios against the synthetic library."""

import re
import time


def instant_mix(tui):
    def select(title):
        tui.key("2", "Escape", "Escape", "Enter", "g", "g", "Enter", "/", *title, "Enter")
        tui.expect(title)

    def original_mix():
        tui.key("3")
        screen = tui.screen()
        for title in ["Seed", "Similar A", "Similar B"]:
            assert title in screen, f"Queue lost {title!r}"
        assert "Empty" not in screen and "Slow" not in screen

    select("Seed")
    tui.key("m")
    tui.expect("Instant Mix: playing 3 tracks")
    original_mix()
    tui.key("n")
    deadline = time.monotonic() + 5
    while not re.search(r"^Similar A\s{2,}", tui.screen(), re.MULTILINE):
        assert time.monotonic() < deadline, "Next-track playback did not start"
        time.sleep(0.1)

    for title, message in [("Empty", "Instant Mix: no similar songs"),
                           ("Error", "Instant Mix failed:")]:
        select(title)
        tui.key("m")
        tui.expect(message)
        original_mix()

    select("Slow")
    tui.key("m")
    tui.expect("Instant Mix: fetching")
    tui.key("m")
    tui.expect("Instant Mix cancelled")
    time.sleep(5.2)
    original_mix()

    select("Slow")
    tui.key("m")
    tui.expect("Instant Mix: fetching")
    tui.key("a")
    tui.expect("Instant Mix cancelled: queue or playback changed")
    tui.key("3")
    assert "Slow" in tui.screen() and "Seed" in tui.screen()

    tui.key("g", "g", "m")
    tui.expect("Instant Mix: playing 3 tracks")
    original_mix()
    tui.key("p")
    tui.quit()
    assert [song["id"] for song in tui.state()["queue"]] == ["seed", "mix-a", "mix-b"]
    assert tui.state()["player_volume"] == 0
