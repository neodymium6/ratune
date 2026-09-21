//! Artist shelf → album covers → track list, sharing the existing library selections.
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use ratatui_image::picker::ProtocolType;

use crate::{
    app::{App, BrowserColumn},
    state::LoadingState,
    text_width::truncate_to_width,
};

#[derive(Clone, Copy)]
pub struct BrowserLayout {
    pub artists: Rect,
    pub content: Rect,
    pub header: Rect,
    pub footer: Rect,
}

pub fn layout(area: Rect) -> BrowserLayout {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    let width = (area.width / 4).clamp(12, 36).min(area.width / 2);
    let cols = Layout::horizontal([Constraint::Length(width), Constraint::Min(0)]).split(rows[1]);
    BrowserLayout {
        artists: cols[0],
        content: cols[1],
        header: rows[0],
        footer: rows[2],
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub area: Rect,
    pub columns: usize,
    pub rows: usize,
    pub card_width: u16,
    pub card_height: u16,
}

impl Grid {
    pub fn new(area: Rect) -> Self {
        let columns = (area.width as usize / 28).clamp(1, 5);
        let rows = (area.height as usize / 17).clamp(1, 3).min(12 / columns);
        Self {
            area,
            columns,
            rows,
            card_width: area.width / columns as u16,
            card_height: (area.height / rows as u16).min(19),
        }
    }
    pub fn capacity(self) -> usize {
        self.columns * self.rows
    }
    pub fn card(self, slot: usize) -> Rect {
        Rect::new(
            self.area.x + (slot % self.columns) as u16 * self.card_width,
            self.area.y + (slot / self.columns) as u16 * self.card_height,
            self.card_width,
            self.card_height,
        )
    }
    pub fn scroll(self, selected: usize, len: usize, old: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let selected = selected.min(len - 1);
        let mut first = (old / self.columns).min((len - 1) / self.columns) * self.columns;
        if selected < first {
            first = selected / self.columns * self.columns;
        }
        if selected >= first + self.capacity() {
            first = (selected / self.columns + 1 - self.rows) * self.columns;
        }
        first
    }
}

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let parts = layout(area);
    let artist = app
        .library
        .current_artist()
        .map(|a| a.name.as_str())
        .unwrap_or("Artists");
    let header = format!(" Library  /  {artist}");
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .fg(app.accent())
                .add_modifier(Modifier::BOLD),
        ),
        parts.header,
    );
    super::artists::render(
        app,
        frame,
        parts.artists,
        app.browser_focus == BrowserColumn::Artists,
    );
    if app.browser_focus == BrowserColumn::Tracks {
        super::tracks::render(app, frame, parts.content, true);
    } else {
        render_albums(app, frame, parts.content);
    }
    let hints = match app.browser_focus {
        BrowserColumn::Artists => " j/k: artist   Enter/l: albums   /: filter artists",
        BrowserColumn::Albums => " h/l: album   j/k: row   Enter: tracks   Esc: artists   /: filter   a: queue album   Ctrl+r: play album",
        BrowserColumn::Tracks => " j/k: track   Enter/a: queue track   A: queue album   Ctrl+r: play album   m: Instant Mix   Esc/h: albums",
    };
    frame.render_widget(
        Paragraph::new(hints).style(Style::default().fg(app.theme.dimmed)),
        parts.footer,
    );
    app.browser_art.finish_frame(frame);
}

pub fn visible_albums(app: &App) -> Vec<usize> {
    let Some(artist) = app.library.current_artist() else {
        return Vec::new();
    };
    let Some(LoadingState::Loaded(albums)) = app.library.albums.get(&artist.id) else {
        return Vec::new();
    };
    let query = app.browser_column_filter(BrowserColumn::Albums);
    albums
        .iter()
        .enumerate()
        .filter(|(_, a)| query.is_none_or(|q| a.name.to_lowercase().contains(q)))
        .map(|(i, _)| i)
        .collect()
}

fn render_albums(app: &mut App, frame: &mut Frame, area: Rect) {
    app.browser_album_hits.clear();
    let indices = visible_albums(app);
    let albums = app
        .library
        .current_artist()
        .and_then(|artist| app.library.albums.get(&artist.id));
    let Some(LoadingState::Loaded(albums)) = albums else {
        super::albums::render(app, frame, area, app.browser_focus == BrowserColumn::Albums);
        return;
    };
    let active = app.browser_focus == BrowserColumn::Albums;
    let selected = app
        .library
        .selected_album
        .and_then(|s| indices.iter().position(|i| *i == s))
        .unwrap_or(0);
    let title = format!(
        " Albums · {}{} ",
        indices.len(),
        if indices.is_empty() {
            String::new()
        } else {
            format!(" · {}/{}", selected + 1, indices.len())
        }
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if active {
            app.accent()
        } else {
            app.theme.border
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let grid = Grid::new(inner);
    app.browser_album_columns = grid.columns;
    if active {
        app.browser_list_viewport_rows = grid.capacity();
    }
    app.library.albums_scroll = grid.scroll(selected, indices.len(), app.library.albums_scroll);
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new("No albums match this filter")
                .style(Style::default().fg(app.theme.dimmed)),
            inner,
        );
        return;
    }
    let cards: Vec<_> = indices
        .iter()
        .skip(app.library.albums_scroll)
        .take(grid.capacity())
        .map(|&i| (i, albums[i].clone()))
        .collect();
    let graphics = !app.help_visible
        && !app.playlist_overlay.visible
        && app.playlist_picker.is_none()
        && !app.favorites_overlay.visible
        && !app.radio.picker_visible
        && app
            .art_picker
            .as_ref()
            .is_some_and(|p| p.protocol_type() == ProtocolType::Iterm2);
    let font = app
        .art_picker
        .as_ref()
        .map(|p| p.font_size())
        .unwrap_or((10, 20));
    for (slot, (index, album)) in cards.into_iter().enumerate() {
        let card = grid.card(slot);
        if card.width < 3 || card.height < 4 {
            continue;
        }
        app.browser_album_hits.push((card, index));
        let selected = app.library.selected_album == Some(index);
        let border = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if selected && active {
                app.accent()
            } else {
                app.theme.border
            }));
        let content = border.inner(card);
        frame.render_widget(border, card);
        let art_height = content.height.saturating_sub(3);
        let art_area = Rect::new(
            content.x + 1,
            content.y,
            content.width.saturating_sub(2),
            art_height,
        );
        let cover = album.cover_art.as_deref().unwrap_or(&album.id);
        let ready = graphics
            && app.browser_art.thumbnail(
                frame,
                cover,
                art_area,
                font,
                app.in_tmux,
                app.server_reachable,
                &app.subsonic,
            );
        if !ready && art_height > 0 {
            frame.render_widget(
                Paragraph::new("♫")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(app.theme.dimmed)),
                Rect::new(art_area.x, art_area.y + art_height / 2, art_area.width, 1),
            );
        }
        let title = truncate_to_width(&album.name, content.width as usize);
        let artist = truncate_to_width(
            album.artist.as_deref().unwrap_or(""),
            content.width as usize,
        );
        let meta = format!(
            "{}{}",
            album.year.map(|y| y.to_string()).unwrap_or_default(),
            album
                .song_count
                .map(|n| format!("  ·  {n} tracks"))
                .unwrap_or_default()
        );
        let lines = vec![
            Line::from(title).style(
                Style::default()
                    .fg(if selected && active {
                        app.accent()
                    } else {
                        app.theme.foreground
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(artist).style(Style::default().fg(app.theme.dimmed)),
            Line::from(meta).style(Style::default().fg(app.theme.dimmed)),
        ];
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            Rect::new(
                content.x,
                content.y + art_height,
                content.width,
                3.min(content.height),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_scroll_is_row_aligned_and_keeps_selection_visible() {
        let grid = Grid::new(Rect::new(2, 2, 100, 40));
        assert_eq!(grid.columns, 3);
        assert_eq!(grid.rows, 2);
        for selected in 0..40 {
            let start = grid.scroll(selected, 40, 27);
            assert_eq!(start % grid.columns, 0);
            assert!(start <= selected && selected < start + grid.capacity());
        }
        assert_eq!(grid.scroll(0, 0, 99), 0);
    }
    #[test]
    fn cards_and_panes_stay_in_bounds_even_in_small_terminals() {
        for (w, h) in [(0, 0), (20, 8), (80, 24), (214, 60)] {
            let area = Rect::new(0, 0, w, h);
            let parts = layout(area);
            assert!(parts.content.right() <= area.right());
            assert!(parts.content.bottom() <= area.bottom());
            let grid = Grid::new(parts.content);
            for i in 0..grid.capacity() {
                let card = grid.card(i);
                assert!(card.right() <= parts.content.right());
                assert!(card.bottom() <= parts.content.bottom());
            }
            assert!(grid.capacity() <= 12);
        }
    }
}
