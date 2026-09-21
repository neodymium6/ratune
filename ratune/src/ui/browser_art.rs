//! Bounded, asynchronous thumbnails for the album browser (iTerm2 / tmux).

use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    fmt::Write as _,
    hash::{Hash, Hasher},
    io::{self, Write},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use base64::Engine;
use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    QueueableCommand,
};
use ratatui::{layout::Rect, Frame};
use ratatui_image::protocol::iterm2::Iterm2;
use ratune_subsonic::SubsonicClient;

const CACHE_LIMIT: usize = 48;
const MAX_IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    cover: String,
    width: u16,
    height: u16,
    font: (u16, u16),
    tmux: bool,
}

struct Entry {
    image: Option<Iterm2>,
    used: u64,
    retry_after: Instant,
}

pub struct BrowserArt {
    cache: HashMap<Key, Entry>,
    pending: HashSet<Key>,
    tx: mpsc::Sender<(Key, Option<Iterm2>)>,
    rx: mpsc::Receiver<(Key, Option<Iterm2>)>,
    visible: Vec<(Key, Rect)>,
    displayed: HashMap<(Key, Rect), u64>,
    row_text: Vec<u64>,
    tick: u64,
}

impl Default for BrowserArt {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
            tx,
            rx,
            visible: Vec::new(),
            displayed: HashMap::new(),
            row_text: Vec::new(),
            tick: 0,
        }
    }
}

impl BrowserArt {
    pub fn begin_frame(&mut self) {
        self.visible.clear();
        self.tick = self.tick.wrapping_add(1);
        while let Ok((key, image)) = self.rx.try_recv() {
            self.pending.remove(&key);
            self.cache.insert(
                key,
                Entry {
                    image,
                    used: self.tick,
                    retry_after: Instant::now() + Duration::from_secs(60),
                },
            );
        }
        self.trim();
    }

    fn trim(&mut self) {
        while self.cache.len() > CACHE_LIMIT {
            let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.cache.remove(&oldest);
        }
    }

    /// Invalidate terminal placements, not decoded/encoded cache entries.
    pub fn invalidate(&mut self) {
        self.displayed.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn thumbnail(
        &mut self,
        frame: &mut Frame,
        cover: &str,
        bounds: Rect,
        font: (u16, u16),
        tmux: bool,
        online: bool,
        client: &Arc<SubsonicClient>,
    ) -> bool {
        if bounds.is_empty() || font.0 == 0 || font.1 == 0 {
            return false;
        }
        let key = Key {
            cover: cover.into(),
            width: bounds.width,
            height: bounds.height,
            font,
            tmux,
        };
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.used = self.tick;
            if let Some(image) = &entry.image {
                let area = Rect::new(
                    bounds.x + (bounds.width.saturating_sub(image.area.width)) / 2,
                    bounds.y + (bounds.height.saturating_sub(image.area.height)) / 2,
                    image.area.width.min(bounds.width),
                    image.area.height.min(bounds.height),
                );
                for y in area.top()..area.bottom() {
                    for x in area.left()..area.right() {
                        frame.buffer_mut()[(x, y)].set_skip(true);
                    }
                }
                self.visible.push((key, area));
                return true;
            }
            if Instant::now() < entry.retry_after {
                return false;
            }
        }
        if online && self.pending.len() < MAX_IN_FLIGHT && self.pending.insert(key.clone()) {
            let client = Arc::clone(client);
            let tx = self.tx.clone();
            tokio::spawn(async move {
                // Small source, bounded concurrency; decode/resize/PNG never run on the UI thread.
                let image = match client.get_cover_art_sized(&key.cover, 384).await {
                    Ok(bytes) => {
                        let encoding_key = key.clone();
                        tokio::task::spawn_blocking(move || encode(&bytes, &encoding_key))
                            .await
                            .ok()
                            .flatten()
                    }
                    Err(_) => None, // Do not log signed URLs or credentials.
                };
                let _ = tx.send((key, image));
            });
        }
        false
    }

    pub fn finish_frame(&mut self, frame: &mut Frame) {
        // tmux can repaint full rows when the artist list scrolls over wide text.
        // Hash text on image rows, ignoring styles and protected image cells.
        let area = frame.area();
        self.row_text.clear();
        self.row_text.resize(area.bottom() as usize, 0);
        for y in area.top()..area.bottom() {
            if self
                .visible
                .iter()
                .any(|(_, r)| y >= r.top() && y < r.bottom())
            {
                let mut hash = DefaultHasher::new();
                for x in area.left()..area.right() {
                    let cell = &frame.buffer_mut()[(x, y)];
                    if !cell.skip {
                        x.hash(&mut hash);
                        cell.symbol().hash(&mut hash);
                    }
                }
                self.row_text[y as usize] = hash.finish();
            }
        }
    }

    fn text_key(&self, area: Rect) -> u64 {
        let mut hash = DefaultHasher::new();
        for y in area.top()..area.bottom() {
            self.row_text.get(y as usize).hash(&mut hash);
        }
        hash.finish()
    }

    pub fn draw(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if self.visible.is_empty() {
            self.invalidate();
            return Ok(());
        }
        let mut wrote = false;
        for (key, area) in &self.visible {
            let placement = (key.clone(), *area);
            let text_key = self.text_key(*area);
            if self.displayed.get(&placement) == Some(&text_key) {
                continue;
            }
            if let Some(image) = self.cache.get(key).and_then(|e| e.image.as_ref()) {
                if !wrote {
                    writer.queue(SavePosition)?;
                    wrote = true;
                }
                writer.queue(MoveTo(area.x, area.y))?;
                writer.write_all(image.data.as_bytes())?;
                self.displayed.insert(placement, text_key);
            }
        }
        if wrote {
            writer.queue(RestorePosition)?.flush()?;
        }
        self.displayed
            .retain(|placement, _| self.visible.contains(placement));
        Ok(())
    }
}

fn encode(bytes: &[u8], key: &Key) -> Option<Iterm2> {
    let img = image::load_from_memory(bytes).ok()?;
    let bounds = Rect::new(0, 0, key.width, key.height);
    let img = super::art_prepare::prepare_art_image_for_rect_contain_fit(img, bounds, key.font);
    let area = super::art_prepare::contain_fit_rect_in_cells(&img, bounds, key.font);
    let area = Rect::new(0, 0, area.width, area.height);
    let png = Iterm2::new(img.clone(), area, key.tmux).ok()?;
    // Album art is usually photographic JPEG. Re-encoding it losslessly as PNG
    // makes each scroll send hundreds of kilobytes per cover. Keep PNG for true
    // transparency and flat artwork; otherwise choose the smaller wire payload.
    if img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p[3] != 255) {
        return Some(png);
    }
    let mut jpeg = Vec::new();
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&image::DynamicImage::ImageRgb8(img.to_rgb8()))
        .is_err()
    {
        return Some(png);
    }
    let data = inline_file(&jpeg, img.width(), img.height(), area, key.tmux);
    if data.len() >= png.data.len() {
        return Some(png);
    }
    Some(Iterm2 {
        data,
        area,
        is_tmux: key.tmux,
    })
}

fn inline_file(bytes: &[u8], width: u32, height: u32, area: Rect, tmux: bool) -> String {
    // Match ratatui-image's erase/position framing, with a JPEG payload. This
    // preserves transparent-image cleanup, tmux escaping and pixel geometry.
    let (start, escape, end) = if tmux {
        ("\x1bPtmux;", "\x1b\x1b", "\x1b\\")
    } else {
        ("", "\x1b", "")
    };
    let mut data = String::from(start);
    for _ in 0..area.height {
        write!(data, "{escape}[{}X{escape}[1B", area.width).unwrap();
    }
    write!(data, "{escape}[{}A", area.height).unwrap();
    write!(data, "{escape}]1337;File=inline=1;size={};width={width}px;height={height}px;doNotMoveCursor=1:{}\x07{end}",
        bytes.len(), base64::engine::general_purpose::STANDARD.encode(bytes)).unwrap();
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(i: usize) -> Key {
        Key {
            cover: i.to_string(),
            width: 20,
            height: 10,
            font: (10, 20),
            tmux: true,
        }
    }

    #[test]
    fn idle_frames_reuse_encoding_and_focus_only_resends() {
        let mut art = BrowserArt::default();
        let key = key(0);
        art.cache.insert(
            key.clone(),
            Entry {
                image: Some(Iterm2 {
                    data: "1337;File=cached".into(),
                    ..Iterm2::default()
                }),
                used: 0,
                retry_after: Instant::now(),
            },
        );
        art.visible.push((key, Rect::new(1, 1, 20, 10)));
        let mut out = Vec::new();
        art.draw(&mut out).unwrap();
        assert!(out.windows(10).any(|s| s == b"1337;File="));
        out.clear();
        art.draw(&mut out).unwrap();
        assert!(out.is_empty());
        art.invalidate();
        art.draw(&mut out).unwrap();
        assert!(!out.is_empty());
        assert_eq!(art.cache.len(), 1);
        out.clear();
        art.row_text.resize(12, 1);
        art.draw(&mut out).unwrap();
        assert!(!out.is_empty(), "repair after neighboring text scrolls");
    }

    #[test]
    fn cache_is_bounded_and_retains_recent_entries() {
        let mut art = BrowserArt::default();
        for i in 0..60 {
            art.cache.insert(
                key(i),
                Entry {
                    image: None,
                    used: i as u64,
                    retry_after: Instant::now(),
                },
            );
        }
        art.trim();
        assert_eq!(art.cache.len(), CACHE_LIMIT);
        assert!(art.cache.contains_key(&key(59)));
        assert!(!art.cache.contains_key(&key(0)));
    }

    fn two_shelves() -> BrowserArt {
        let mut art = BrowserArt::default();
        for i in 0..13 {
            art.cache.insert(
                key(i),
                Entry {
                    image: Some(Iterm2 {
                        data: format!("image-{i};"),
                        ..Iterm2::default()
                    }),
                    used: 0,
                    retry_after: Instant::now(),
                },
            );
            if i < 12 {
                art.visible.push((
                    key(i),
                    Rect::new(1 + (i % 6) as u16 * 22, 1 + (i / 6) as u16 * 15, 20, 10),
                ));
            }
        }
        art
    }

    #[test]
    fn replacing_one_cover_does_not_retransmit_eleven_unchanged_images() {
        let mut art = two_shelves();
        art.draw(&mut Vec::new()).unwrap();
        art.visible[0].0 = key(12);
        let mut output = Vec::new();
        art.draw(&mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output.matches("image-").count(),
            1,
            "unrelated covers must remain cached on the terminal"
        );
        assert!(output.contains("image-12;"));
    }

    #[test]
    fn scrolling_upper_shelf_does_not_retransmit_lower_shelf() {
        let mut art = two_shelves();
        art.draw(&mut Vec::new()).unwrap();
        for i in 0..6 {
            art.visible[i].0 = key((i + 1) % 6);
        }
        let mut output = Vec::new();
        art.draw(&mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("image-").count(), 6);
        for i in 6..12 {
            assert!(!output.contains(&format!("image-{i};")));
        }
    }

    #[test]
    fn neighboring_text_repairs_only_images_on_affected_rows() {
        use ratatui::{backend::TestBackend, widgets::Paragraph, Terminal};
        let mut terminal = Terminal::new(TestBackend::new(150, 32)).unwrap();
        let mut art = two_shelves();
        for text in ["artist", "別のアーティスト"] {
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new(text), Rect::new(135, 2, 15, 1));
                    art.finish_frame(frame);
                })
                .unwrap();
            let mut output = Vec::new();
            art.draw(&mut output).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(
                output.matches("image-").count(),
                if text == "artist" { 12 } else { 6 }
            );
        }
    }

    #[test]
    fn encode_fits_large_covers_and_rejects_invalid_data() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(600, 600)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let image = encode(bytes.get_ref(), &key(0)).unwrap();
        assert_eq!(image.area, Rect::new(0, 0, 20, 10));
        assert!(image.data.contains("width=200px;height=200px"));
        assert!(encode(b"invalid image", &key(0)).is_none());
    }

    #[test]
    fn photographic_thumbnails_use_less_than_half_the_lossless_payload() {
        let mut noise = 42_u32;
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(384, 384, |x, y| {
            noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
            let grain = ((noise >> 24) & 31) as u8;
            image::Rgb([
                (x * 220 / 384) as u8 + grain,
                (y * 220 / 384) as u8 + grain,
                ((x + y) * 110 / 384) as u8 + grain,
            ])
        }));
        let mut input = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut input, 90)
            .encode_image(&img)
            .unwrap();
        let key = Key {
            width: 30,
            height: 16,
            font: (14, 27),
            ..key(0)
        };
        let decoded = image::load_from_memory(&input).unwrap();
        let legacy = Iterm2::new(decoded, Rect::new(0, 0, 28, 15), true).unwrap();
        let compact = encode(&input, &key).unwrap();
        eprintln!(
            "thumbnail transport: lossless={}B compact={}B",
            legacy.data.len(),
            compact.data.len()
        );
        assert!(compact.data.len() * 2 < legacy.data.len());
        assert!(compact.data.contains("width=384px;height=384px"));
        assert_eq!(compact.area, Rect::new(0, 0, 28, 15));
        let payload = decode_inline(&compact);
        assert_eq!(
            image::guess_format(&payload).unwrap(),
            image::ImageFormat::Jpeg
        );
        let decoded = image::load_from_memory(&payload).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (384, 384));
    }

    fn decode_inline(image: &Iterm2) -> Vec<u8> {
        let payload = image
            .data
            .split("doNotMoveCursor=1:")
            .nth(1)
            .unwrap()
            .split('\x07')
            .next()
            .unwrap();
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap()
    }

    #[test]
    fn compact_transport_preserves_transparency_and_small_pngs() {
        for alpha in [100, 255] {
            let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                200,
                200,
                image::Rgba([240, 90, 20, alpha]),
            ));
            let mut input = std::io::Cursor::new(Vec::new());
            img.write_to(&mut input, image::ImageFormat::Png).unwrap();
            let encoded = encode(input.get_ref(), &key(0)).unwrap();
            let payload = decode_inline(&encoded);
            assert_eq!(
                image::guess_format(&payload).unwrap(),
                image::ImageFormat::Png
            );
            assert_eq!(
                image::load_from_memory(&payload)
                    .unwrap()
                    .to_rgba8()
                    .get_pixel(0, 0)[3],
                alpha
            );
        }
    }

    #[test]
    fn compact_inline_file_uses_exact_tmux_framing_and_size() {
        let bytes = b"test payload";
        for tmux in [false, true] {
            let data = inline_file(bytes, 100, 80, Rect::new(0, 0, 10, 4), tmux);
            assert_eq!(data.contains("\x1bPtmux;"), tmux);
            assert_eq!(data.contains("\x1b\x1b]1337"), tmux);
            assert!(data.contains(";size=12;width=100px;height=80px;doNotMoveCursor=1:"));
            assert!(data.ends_with(if tmux { "\x07\x1b\\" } else { "\x07" }));
            assert_eq!(
                decode_inline(&Iterm2 {
                    data,
                    ..Iterm2::default()
                }),
                bytes
            );
        }
    }

    #[test]
    fn text_key_ignores_styles_but_detects_neighboring_text_changes() {
        use ratatui::{
            backend::TestBackend,
            style::{Color, Style},
            widgets::Paragraph,
            Terminal,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut art = BrowserArt::default();
        art.visible.push((key(0), Rect::new(30, 1, 20, 10)));
        let mut keys = Vec::new();
        for (text, color) in [
            ("Artist", Color::White),
            ("Artist", Color::Cyan),
            ("別のArtist", Color::Cyan),
        ] {
            terminal
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(text).style(Style::default().fg(color)),
                        Rect::new(1, 1, 20, 1),
                    );
                    art.finish_frame(frame);
                })
                .unwrap();
            keys.push(art.text_key(Rect::new(30, 1, 20, 10)));
        }
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
    }
}
