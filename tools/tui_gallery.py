"""Gallery navigation against a multi-album synthetic library."""

import re
import time


def gallery(tui):
    tui.key("2", "Escape", "Escape", "Enter", "g", "g")
    tui.expect("1/19")
    width = int(tui.tmux("display-message", "-p", "-t", "fixture:0.0", "#{pane_width}"))
    columns = max(1, min(5, (width - min(max(width // 4, 12), 36) - 2) // 28))
    tui.key("l")
    tui.expect("2/19")
    tui.key("j")
    tui.expect(f"{2+columns}/19")
    tui.key("Enter")
    tui.expect(f"Gallery {1+columns:02}")
    tui.expect("Gallery track 1")
    tui.key("Escape")
    tui.expect(f"{2+columns}/19")

    tui.key("G")
    tui.expect("19/19")
    tui.key("k")
    tui.expect(f"{19-columns}/19")
    tui.key("g", "g")
    tui.expect("1/19")

    tui.key("/", *list("no-such-album"), "Enter")
    tui.expect("No albums match this filter")
    tui.key("Escape")
    tui.expect("1/19")

    output = tui.screen()
    album_border_row = next(i for i, line in enumerate(output.splitlines()) if "Albums · 19" in line)
    left_width = min(max(width // 4, 12), 36)
    card_width = (width - left_width - 2) // columns
    # Click inside the second visible card (terminal SGR mouse coordinates are 1-based).
    x, y = left_width + 2 + card_width + 2, album_border_row + 4
    tui.tmux("send-keys", "-t", "fixture:0.0", "-l", f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m")
    time.sleep(0.3)
    tui.expect("2/19")

    tui.key("3")
    old_count = int(re.search(r"Queue \((\d+)\)", tui.screen()).group(1))
    tui.key("2", "a", "3")
    tui.expect(f"Queue ({old_count+3})")
    tui.key("2", "C-r", "3")
    tui.expect("Queue (3)")
    tui.expect("Gallery track 1")
    tui.key("2", "Escape", "Escape", "Enter", "g", "g")
    tui.quit()
