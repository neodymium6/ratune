//! Now Playing's cached iTerm2 image, composited after the text frame.

use std::io::{self, Write};

use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    QueueableCommand,
};
use ratatui::{layout::Rect, Frame};
use ratatui_image::{protocol::iterm2::Iterm2, Resize, ResizeEncodeRender};

pub fn prepare(frame: &mut Frame, state: &mut impl ResizeEncodeRender, area: Rect, resize: Resize) {
    if let Some(resize_area) = state.needs_resize(&resize, area) {
        state.resize_encode(&resize, resize_area);
    }
    // Do not put a long inline-image escape sequence into the text buffer:
    // it confuses ratatui's Unicode width/diff logic. The cached image will be
    // sent after all text, including tmux's full-line redraws, is finished.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.set_skip(true);
            }
        }
    }
}

#[derive(Default)]
pub struct Iterm2Overlay {
    last: Option<(Rect, u64, Option<u64>)>,
}

impl Iterm2Overlay {
    pub fn draw(
        &mut self,
        writer: &mut impl Write,
        image: Option<(&Iterm2, Rect, u64)>,
        queue_text_key: Option<u64>,
    ) -> io::Result<()> {
        let Some((image, area, fingerprint)) = image
            .filter(|(image, area, _)| !image.data.is_empty() && area.width > 0 && area.height > 0)
        else {
            self.last = None;
            return Ok(());
        };
        let key = (area, fingerprint, queue_text_key);
        if self.last == Some(key) {
            return Ok(());
        }
        writer.queue(SavePosition)?.queue(MoveTo(area.x, area.y))?;
        writer.write_all(image.data.as_bytes())?;
        writer.queue(RestorePosition)?;
        writer.flush()?;
        self.last = Some(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    use image::{DynamicImage, Rgba};
    use ratatui::{
        backend::CrosstermBackend,
        style::{Color, Style},
        widgets::Paragraph,
        Terminal, TerminalOptions, Viewport,
    };
    use ratatui_image::protocol::{ImageSource, StatefulProtocol, StatefulProtocolType};

    #[derive(Clone, Default)]
    struct Output(Rc<RefCell<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn cover() -> StatefulProtocol {
        StatefulProtocol::new(
            ImageSource::new(DynamicImage::new_rgb8(64, 64), (8, 16), Rgba([0; 4])),
            (8, 16),
            StatefulProtocolType::ITerm2(Iterm2 {
                is_tmux: true,
                ..Iterm2::default()
            }),
        )
    }

    struct Harness {
        terminal: Terminal<CrosstermBackend<Output>>,
        output: Output,
        cover: StatefulProtocol,
        overlay: Iterm2Overlay,
    }
    impl Harness {
        fn new() -> Self {
            let output = Output::default();
            let terminal = Terminal::with_options(
                CrosstermBackend::new(output.clone()),
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 80, 20)),
                },
            )
            .unwrap();
            Self {
                terminal,
                output,
                cover: cover(),
                overlay: Iterm2Overlay::default(),
            }
        }

        fn draw(&mut self, queue: &str, key: u64, selected: bool, visible: bool) -> String {
            let area = Rect::new(1, 1, 8, 4);
            self.terminal
                .draw(|frame| {
                    if visible {
                        prepare(frame, &mut self.cover, area, Resize::Scale(None));
                    }
                    frame.render_widget(
                        Paragraph::new(queue).style(Style::default().fg(if selected {
                            Color::Cyan
                        } else {
                            Color::White
                        })),
                        Rect::new(20, 1, 55, 1),
                    );
                })
                .unwrap();
            let image = if visible {
                match self.cover.protocol_type() {
                    StatefulProtocolType::ITerm2(encoded) => Some((encoded, area, 123)),
                    _ => None,
                }
            } else {
                None
            };
            self.overlay
                .draw(self.terminal.backend_mut(), image, Some(key))
                .unwrap();
            String::from_utf8(self.output.0.take()).unwrap()
        }
    }

    #[test]
    fn image_is_drawn_after_queue_text() {
        let mut h = Harness::new();
        let output = h.draw("Queue 日本語", 1, false, true);
        let image = output.find("1337;File=").unwrap();
        let queue = output.find("Queue").unwrap();
        assert!(
            image > queue,
            "queue text must be finished before the image is sent"
        );
    }

    #[test]
    fn scrolling_queue_repaints_art_after_text_without_reencoding() {
        let mut h = Harness::new();
        h.draw("Queue 日本語", 1, false, true);
        h.cover.last_encoding_result();
        let output = h.draw("Scrolled 英語", 2, false, true);
        assert_eq!(output.matches("1337;File=").count(), 1);
        assert!(output.find("1337;File=").unwrap() > output.find("Scrolled").unwrap());
        assert!(
            h.cover.last_encoding_result().is_none(),
            "reuse cached encoding"
        );
    }

    #[test]
    fn cursor_style_and_idle_frames_do_not_retransmit() {
        let mut h = Harness::new();
        h.draw("Queue 日本語", 1, false, true);
        for selected in [true, false, false] {
            let output = h.draw("Queue 日本語", 1, selected, true);
            assert_eq!(output.matches("1337;File=").count(), 0);
        }
    }

    #[test]
    fn tab_help_or_focus_gap_repaints_art() {
        let mut h = Harness::new();
        h.draw("Queue 日本語", 1, false, true);
        h.draw("Queue 日本語", 1, false, false);
        assert_eq!(
            h.draw("Queue 日本語", 1, false, true)
                .matches("1337;File=")
                .count(),
            1
        );
    }

    #[test]
    fn new_cover_and_placement_redraw_and_restore_cursor() {
        let mut h = Harness::new();
        h.draw("Queue", 1, false, true);
        let StatefulProtocolType::ITerm2(image) = h.cover.protocol_type() else {
            unreachable!()
        };
        for (area, fingerprint) in [(Rect::new(1, 1, 8, 4), 456), (Rect::new(2, 2, 8, 4), 456)] {
            h.overlay
                .draw(&mut h.output, Some((image, area, fingerprint)), Some(1))
                .unwrap();
            let output = String::from_utf8(h.output.0.take()).unwrap();
            assert_eq!(output.matches("1337;File=").count(), 1);
            assert!(output.starts_with("\x1b7"));
            assert!(output.ends_with("\x1b8"));
        }
    }
}
