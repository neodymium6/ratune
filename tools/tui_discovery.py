"""Discovery shelves and guarded album actions using synthetic history."""

import re
import time


def discovery(tui):
    def queue_count():
        tui.key("3")
        return int(re.search(r"Queue \((\d+)\)", tui.screen()).group(1))

    original_count = queue_count()
    tui.key("1", "g", "g")
    tui.expect("Recently Added · 1/19")
    tui.expect("Rediscover · local listening history")
    tui.expect("Start a Mix · recent tracks")
    tui.expect("Gallery 18")
    tui.key("l")
    tui.expect("Recently Added · 2/19")
    tui.key("2", "1")
    tui.expect("Recently Added · 2/19")
    tui.key("r")
    tui.expect("Recently Added · 2/19")

    tui.key("g", "g", "Enter")
    tui.expect("Cannot load album — queue unchanged")
    assert queue_count() == original_count
    tui.key("1", "l", "Enter")
    tui.expect("Album has no tracks — queue unchanged")
    assert queue_count() == original_count

    tui.key("1", "l", "Enter")
    tui.expect("Loading selected album")
    tui.key("Escape")
    tui.expect("Album action cancelled")
    time.sleep(5.2)
    assert queue_count() == original_count
    tui.key("1", "Enter")
    tui.expect("Loading selected album")
    tui.key("3", "d")
    tui.expect("Album action cancelled: queue or playback changed")
    assert queue_count() == original_count - 1

    tui.key("1", "l", "a")
    tui.expect("Album: queued 3 tracks")
    assert queue_count() == original_count + 2
    tui.key("1", "Enter")
    tui.expect("Album: playing 3 tracks")
    assert queue_count() == 3
    tui.expect("Gallery track 1")
    tui.key("1", "C-r")
    tui.expect("Album: playing 3 tracks")
    assert queue_count() == 3

    tui.key("1", "J", "J")
    tui.expect("Enter/m: start Mix")
    assert "Your recently played tracks will appear here" not in tui.screen(), "Synthetic history is required"
    tui.key("g", "g", "m")
    tui.expect("Instant Mix: playing 3 tracks")
    assert queue_count() == 3
    tui.expect("Similar A")
    tui.expect("Similar B")

    tui.key("1", "K", "K", "g", "g")
    output = tui.screen()
    row = next(i for i, line in enumerate(output.splitlines()) if "Recently Added ·" in line)
    width = int(tui.tmux("display-message", "-p", "-t", "fixture:0.0", "#{pane_width}"))
    visible = max(1, min(6, (width - 2) // 30))
    card_width = (width - 2) // visible
    tui.tmux("send-keys", "-t", "fixture:0.0", "-l", f"\x1b[<0;{card_width+3};{row+4}M\x1b[<0;{card_width+3};{row+4}m")
    tui.expect("Recently Added · 2/19")
    assert queue_count() == 3
    tui.key("1", "i")
    tui.expect("Home Tab (1)")
    tui.key("Escape")
    tui.expect("Recently Added · 2/19")
    tui.key("3", "p")

    tui.quit()
