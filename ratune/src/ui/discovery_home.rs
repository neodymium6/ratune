//! Home is for deciding what to listen to; Browser remains the library explorer.
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use ratatui_image::picker::ProtocolType;

use crate::{app::App, history::PlayRecord, state::LibraryState, text_width::truncate_to_width};

pub struct DiscoveryLayout {
    pub header: Rect,
    pub shelves: [Rect; 3],
    pub footer: Rect,
}

pub fn layout(area: Rect, active: usize) -> DiscoveryLayout {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);
    let shelves = if area.height < 30 {
        let mut shelves = [Rect::default(); 3];
        shelves[active.min(2)] = rows[1];
        shelves
    } else {
        let bands = Layout::vertical([
            Constraint::Percentage(40),
            Constraint::Percentage(40),
            Constraint::Percentage(20),
        ])
        .split(rows[1]);
        [bands[0], bands[1], bands[2]]
    };
    DiscoveryLayout {
        header: rows[0],
        shelves,
        footer: rows[2],
    }
}

fn shelf_block(app: &App, title: String, section: usize) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_set(app.theme.border_set)
        .border_style(Style::default().fg(if app.discovery.section == section {
            app.accent()
        } else {
            app.theme.border
        }))
}

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    app.home_recent_albums_inner = None;
    app.home_art_needs_redraw = false;
    let parts = layout(area, app.discovery.section);
    let heading = if app.discovery.loading {
        " Discover · refreshing shelves…"
    } else if let Some(error) = app.discovery.error.as_deref() {
        error
    } else {
        " Discover · pick something to listen to"
    };
    frame.render_widget(
        Paragraph::new(heading).style(
            Style::default()
                .fg(app.accent())
                .add_modifier(Modifier::BOLD),
        ),
        parts.header,
    );
    for section in 0..2 {
        render_albums(app, frame, parts.shelves[section], section);
    }
    render_seeds(app, frame, parts.shelves[2]);
    let action_hint = if app.discovery.section == 2 {
        " Enter/m: start Mix (replaces queue)   a: add selected track"
    } else {
        " Enter/Ctrl+r: play album (replaces queue)   a: add selected album"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(
                " J/K: section   h/l or j/k: select   r: refresh   2: library   3: now playing",
            ),
            Line::from(action_hint),
        ])
        .style(Style::default().fg(app.theme.dimmed)),
        parts.footer,
    );
    app.browser_art.finish_frame(frame);
}

fn render_albums(app: &mut App, frame: &mut Frame, area: Rect, section: usize) {
    if area.is_empty() {
        return;
    }
    let count = app.discovery.len(section);
    let name = if section == 0 {
        "Recently Added"
    } else {
        "Rediscover · local listening history"
    };
    let position = if count == 0 {
        String::new()
    } else {
        format!(" · {}/{}", app.discovery.selected[section] + 1, count)
    };
    let block = shelf_block(app, format!(" {name}{position} "), section);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if count == 0 {
        let message = if section == 0 {
            app.discovery
                .error
                .as_deref()
                .unwrap_or(if app.discovery.loading {
                    "Loading new additions…"
                } else {
                    "No albums returned by the server"
                })
        } else if app.discovery.loading {
            "Finding albums…"
        } else {
            "No candidates outside the last 14 days · play more music or press r"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(app.theme.dimmed)),
            inner,
        );
        return;
    }
    let visible = (inner.width as usize / 30).clamp(1, 6);
    app.discovery.visible[section] = visible;
    LibraryState::clamp_vertical_scroll(
        &mut app.discovery.scroll[section],
        app.discovery.selected[section],
        count,
        visible,
    );
    let start = app.discovery.scroll[section];
    let now = PlayRecord::now_secs();
    let cards: Vec<_> = (start..(start + visible).min(count))
        .map(|index| {
            if section == 0 {
                let album = app.discovery.newest[index].clone();
                let info = album
                    .year
                    .map(|y| format!("Released {y}"))
                    .unwrap_or_default();
                (index, album, info)
            } else {
                let item = &app.discovery.rediscover[index];
                let info = item
                    .last_played
                    .map(|t| format!("{} days since last play", (now - t).max(0) / 86400))
                    .unwrap_or_else(|| "Not played in this client".into());
                (index, item.album.clone(), info)
            }
        })
        .collect();
    let graphics = app.config.home_recent_albums_show_art
        && !app.help_visible
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
    let width = inner.width / visible as u16;
    for (slot, (index, album, info)) in cards.into_iter().enumerate() {
        let card = Rect::new(inner.x + slot as u16 * width, inner.y, width, inner.height);
        if card.width < 4 || card.height < 4 {
            continue;
        }
        app.discovery.hits.push((card, section, index));
        let active = app.discovery.section == section && app.discovery.selected[section] == index;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(app.theme.border_set)
            .border_style(Style::default().fg(if active {
                app.accent()
            } else {
                app.theme.border
            }));
        let content = block.inner(card);
        frame.render_widget(block, card);
        let image_height = content.height.saturating_sub(3);
        let image_area = Rect::new(
            content.x + 1,
            content.y,
            content.width.saturating_sub(2),
            image_height,
        );
        let ready = graphics
            && app.browser_art.thumbnail(
                frame,
                album.cover_art.as_deref().unwrap_or(&album.id),
                image_area,
                font,
                app.in_tmux,
                app.server_reachable,
                &app.subsonic,
            );
        if !ready && image_height > 0 {
            frame.render_widget(
                Paragraph::new("♫")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(app.theme.dimmed)),
                Rect::new(
                    image_area.x,
                    image_area.y + image_height / 2,
                    image_area.width,
                    1,
                ),
            );
        }
        let lines = vec![
            Line::from(truncate_to_width(&album.name, content.width as usize)).style(
                Style::default()
                    .fg(if active {
                        app.accent()
                    } else {
                        app.theme.foreground
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(truncate_to_width(
                album.artist.as_deref().unwrap_or(""),
                content.width as usize,
            ))
            .style(Style::default().fg(app.theme.dimmed)),
            Line::from(truncate_to_width(&info, content.width as usize))
                .style(Style::default().fg(app.theme.dimmed)),
        ];
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            Rect::new(
                content.x,
                content.y + image_height,
                content.width,
                3.min(content.height),
            ),
        );
    }
}

fn render_seeds(app: &mut App, frame: &mut Frame, area: Rect) {
    if area.is_empty() {
        return;
    }
    let block = shelf_block(
        app,
        " Start a Mix · recent tracks · Enter/m uses the selected song ".into(),
        2,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.discovery.seeds.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "Your recently played tracks will appear here · start with an album above",
            )
            .style(Style::default().fg(app.theme.dimmed)),
            inner,
        );
        return;
    }
    let len = app.discovery.seeds.len();
    let visible = inner.height.max(1) as usize;
    app.discovery.visible[2] = visible;
    LibraryState::clamp_vertical_scroll(
        &mut app.discovery.scroll[2],
        app.discovery.selected[2],
        len,
        visible,
    );
    for (row, index) in (app.discovery.scroll[2]..len).take(visible).enumerate() {
        let song = &app.discovery.seeds[index];
        let label = format!(
            " {}  —  {}  ·  {}",
            song.title,
            song.artist.as_deref().unwrap_or(""),
            song.album.as_deref().unwrap_or("")
        );
        let selected = app.discovery.section == 2 && app.discovery.selected[2] == index;
        let style = if selected {
            Style::default().fg(app.theme.background).bg(app.accent())
        } else {
            Style::default().fg(app.theme.foreground)
        };
        let rect = Rect::new(inner.x, inner.y + row as u16, inner.width, 1);
        app.discovery.hits.push((rect, 2, index));
        frame.render_widget(
            Paragraph::new(truncate_to_width(&label, inner.width as usize)).style(style),
            rect,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_layout_shows_only_the_active_shelf() {
        for section in 0..3 {
            let layout = layout(Rect::new(0, 0, 60, 20), section);
            assert!(layout.shelves[section].height > 0);
            assert_eq!(layout.shelves.iter().filter(|r| !r.is_empty()).count(), 1);
        }
    }
    #[test]
    fn discovery_shelves_stay_within_terminal() {
        for (w, h) in [(0, 0), (20, 5), (80, 24), (214, 55)] {
            let area = Rect::new(0, 0, w, h);
            let layout = layout(area, 0);
            for rect in [layout.header, layout.footer]
                .into_iter()
                .chain(layout.shelves)
            {
                assert!(rect.right() <= area.right() && rect.bottom() <= area.bottom());
            }
        }
    }
}
