use super::*;

impl App {
    pub(super) fn navigate_album_horizontal(&mut self, right: bool) -> bool {
        if self.active_tab != Tab::Browser
            || self.browser_browse_mode != BrowseMode::Artists
            || self.browser_focus != BrowserColumn::Albums
        {
            return false;
        }
        let indices = crate::ui::browser_gallery::visible_albums(self);
        let current = self
            .library
            .selected_album
            .and_then(|i| indices.iter().position(|&v| v == i))
            .unwrap_or(0);
        let columns = self.browser_album_columns.max(1);
        let target = if right {
            if current % columns + 1 == columns {
                current
            } else {
                current.saturating_add(1)
            }
        } else if current % columns == 0 {
            current
        } else {
            current - 1
        };
        if let Some(&index) = indices.get(target) {
            if Some(index) != self.library.selected_album {
                self.click_browser_album(index);
            }
        }
        true
    }
}
