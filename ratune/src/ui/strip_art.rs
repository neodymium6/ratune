//! Per-thumbnail rendering state for Home's album strip.

use ratatui::{layout::Rect, Frame};
use ratatui_image::{
    protocol::{StatefulProtocol, StatefulProtocolType},
    Resize, ResizeEncodeRender, StatefulImage,
};

pub struct StripArt {
    pub protocol: StatefulProtocol,
    last_frame: Option<(usize, Rect)>,
}

impl StripArt {
    pub fn new(protocol: StatefulProtocol) -> Self {
        Self {
            protocol,
            last_frame: None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, resize: Resize) {
        // ratatui 0.29 counts the inline iTerm2 payload as a very wide symbol.
        // Its Buffer::diff then invalidates later thumbnail cells even when
        // nothing changed, resending their erase + image sequence every frame.
        // Keep unchanged image cells protected, without putting the payload
        // back into the text buffer. A missed frame (tab/help), moved/resized
        // rect or new protocol (cover/focus invalidation) must still repaint.
        let unchanged = matches!(
            self.protocol.protocol_type(),
            StatefulProtocolType::ITerm2(_)
        ) && self
            .last_frame
            .is_some_and(|(last, rect)| last.checked_add(1) == Some(frame.count()) && rect == area)
            && self.protocol.needs_resize(&resize, area).is_none();
        if unchanged {
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                        cell.set_skip(true);
                    }
                }
            }
        } else {
            frame.render_stateful_widget(
                StatefulImage::default().resize(resize),
                area,
                &mut self.protocol,
            );
        }
        self.last_frame = Some((frame.count(), area));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, io, rc::Rc};

    use image::{DynamicImage, Rgba};
    use ratatui::{
        backend::CrosstermBackend, widgets::Paragraph, Terminal, TerminalOptions, Viewport,
    };
    use ratatui_image::protocol::{iterm2::Iterm2, ImageSource, StatefulProtocolType};

    #[derive(Clone, Default)]
    struct Output(Rc<RefCell<Vec<u8>>>);

    impl io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Output {
        fn take_images(&self) -> usize {
            let bytes = self.0.take();
            bytes.windows(10).filter(|w| *w == b"1337;File=").count()
        }
    }

    fn cover(tmux: bool) -> StripArt {
        let font = (8, 16);
        let source = ImageSource::new(DynamicImage::new_rgb8(64, 64), font, Rgba([0; 4]));
        StripArt::new(StatefulProtocol::new(
            source,
            font,
            StatefulProtocolType::ITerm2(Iterm2 {
                is_tmux: tmux,
                ..Iterm2::default()
            }),
        ))
    }

    fn terminal() -> (Terminal<CrosstermBackend<Output>>, Output) {
        let output = Output::default();
        let terminal = Terminal::with_options(
            CrosstermBackend::new(output.clone()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 80, 20)),
            },
        )
        .unwrap();
        (terminal, output)
    }

    fn draw_strip(
        terminal: &mut Terminal<CrosstermBackend<Output>>,
        covers: &mut [StripArt],
        x: u16,
        width: u16,
    ) {
        terminal
            .draw(|frame| {
                for (i, cover) in covers.iter_mut().enumerate() {
                    cover.render(
                        frame,
                        Rect::new(x + i as u16 * 12, 1, width, 4),
                        Resize::Scale(None),
                    );
                }
                frame.render_widget(Paragraph::new("Albums"), Rect::new(1, 6, 30, 1));
            })
            .unwrap();
    }

    #[test]
    fn unchanged_iterm2_strip_does_not_retransmit() {
        for tmux in [false, true] {
            let (mut terminal, output) = terminal();
            let mut covers = [cover(tmux), cover(tmux), cover(tmux)];
            draw_strip(&mut terminal, &mut covers, 1, 8);
            assert_eq!(output.take_images(), 3);
            for _ in 0..5 {
                draw_strip(&mut terminal, &mut covers, 1, 8);
                assert_eq!(output.take_images(), 0, "unchanged strip, tmux={tmux}");
            }
        }
    }

    #[test]
    fn strip_redraws_after_tab_or_help_gap() {
        let (mut terminal, output) = terminal();
        let mut covers = [cover(true), cover(true)];
        draw_strip(&mut terminal, &mut covers, 1, 8);
        output.take_images();
        terminal.draw(|_| {}).unwrap();
        output.take_images();
        draw_strip(&mut terminal, &mut covers, 1, 8);
        assert_eq!(output.take_images(), 2);
        draw_strip(&mut terminal, &mut covers, 1, 8);
        assert_eq!(output.take_images(), 0);
    }

    #[test]
    fn strip_redraws_after_move_resize_and_focus_invalidation() {
        let (mut terminal, output) = terminal();
        let mut covers = [cover(true), cover(true)];
        draw_strip(&mut terminal, &mut covers, 1, 8);
        output.take_images();
        draw_strip(&mut terminal, &mut covers, 2, 8);
        assert_eq!(output.take_images(), 2, "scroll/move");
        draw_strip(&mut terminal, &mut covers, 2, 6);
        assert_eq!(output.take_images(), 2, "resize");
        draw_strip(&mut terminal, &mut covers, 2, 6);
        assert_eq!(output.take_images(), 0);
        // Focus handling clears home_strip_art; even identical bytes must be sent again.
        covers = [cover(true), cover(true)];
        draw_strip(&mut terminal, &mut covers, 2, 6);
        assert_eq!(output.take_images(), 2, "focus invalidation");
    }

    #[test]
    fn replacing_one_cover_does_not_retransmit_its_neighbors() {
        let (mut terminal, output) = terminal();
        let mut covers = [cover(true), cover(true), cover(true)];
        draw_strip(&mut terminal, &mut covers, 1, 8);
        output.take_images();
        covers[1] = cover(true);
        draw_strip(&mut terminal, &mut covers, 1, 8);
        assert_eq!(output.take_images(), 1);
        draw_strip(&mut terminal, &mut covers, 1, 8);
        assert_eq!(output.take_images(), 0);
    }

    #[test]
    fn halfblocks_still_populates_the_buffer_every_frame() {
        use ratatui_image::protocol::halfblocks::Halfblocks;
        let source = ImageSource::new(DynamicImage::new_rgb8(64, 64), (8, 16), Rgba([0; 4]));
        let mut cover = StripArt::new(StatefulProtocol::new(
            source,
            (8, 16),
            StatefulProtocolType::Halfblocks(Halfblocks::default()),
        ));
        let (mut terminal, _) = terminal();
        for _ in 0..3 {
            terminal
                .draw(|frame| {
                    cover.render(frame, Rect::new(1, 1, 8, 4), Resize::Scale(None));
                    assert!(!frame.buffer_mut()[(1, 1)].skip);
                })
                .unwrap();
        }
    }
}
