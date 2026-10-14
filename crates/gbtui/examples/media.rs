//! Run with `cargo run -p gbtui --example media --features images`.

use std::sync::Arc;

use gbtui::ratatui::layout::Rect;
use gbtui::ratatui::widgets::{Block, Paragraph};
use gbtui::{
    App, AppEvent, FrameCtx, HitId, Image, Link, Outcome, Pointer, PreparedImage, ThemeKind,
    ThemeRole, ThemeState, key, run_blocking,
};

const DOCS: HitId = HitId::named("media.docs");
const IMAGE_ID: u32 = 7;

struct MediaApp {
    image: Arc<PreparedImage>,
    link: Link,
    visible: bool,
    moved: bool,
    overlay: bool,
    status: &'static str,
}

impl App for MediaApp {
    type Msg = ();

    fn handle(&mut self, event: AppEvent<()>) -> Outcome {
        match event {
            AppEvent::Key(k) if key!('q').matches(&k) || key!(Esc).matches(&k) => Outcome::Quit,
            AppEvent::Key(k) if key!('i').matches(&k) => {
                self.visible = !self.visible;
                Outcome::Changed
            }
            AppEvent::Key(k) if key!('m').matches(&k) => {
                self.moved = !self.moved;
                Outcome::Changed
            }
            AppEvent::Key(k) if key!('o').matches(&k) => {
                self.overlay = !self.overlay;
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == DOCS => {
                self.status = "Click delivered to the host; no browser was launched.";
                Outcome::Changed
            }
            AppEvent::Hit(_, Pointer::Move { .. })
            | AppEvent::Miss(Pointer::Move { .. })
            | AppEvent::Resize { .. } => Outcome::Changed,
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, frame: &mut gbtui::ratatui::Frame, cx: &mut FrameCtx) {
        let screen = frame.area();
        frame.render_widget(
            Block::default().style(cx.style(ThemeRole::Surface, ThemeState::Normal)),
            screen,
        );
        let title = Rect::new(1, 0, screen.width.saturating_sub(2), screen.height.min(1));
        frame.render_widget(
            Paragraph::new("Media: i hide/show | m move | o cover | q quit"),
            title,
        );
        if screen.height > 2 {
            self.link.render(
                Rect::new(2, 2, screen.width.saturating_sub(4), 1),
                frame.buffer_mut(),
                cx,
            );
        }
        let mut area = Rect::new(
            2,
            4,
            screen.width.saturating_sub(4),
            screen.height.saturating_sub(9),
        );
        if self.moved {
            let shift = (area.width / 3).min(12);
            area.x = area.x.saturating_add(shift);
            area.width = area.width.saturating_sub(shift);
        }
        if self.visible {
            Image::new(self.image.clone(), "[Image: generated RGBA gradient]")
                .id(IMAGE_ID)
                .render(area, frame.buffer_mut(), cx);
        }
        if self.overlay && area.width > 0 && area.height > 0 {
            let cover = Rect::new(
                area.x + area.width / 4,
                area.y + area.height / 4,
                (area.width / 2).max(1),
                (area.height / 2).max(1),
            );
            cx.occlude(cover);
            frame.render_widget(
                Block::bordered()
                    .title("Occluding overlay")
                    .style(cx.style(ThemeRole::Elevated, ThemeState::Normal)),
                cover,
            );
        }
        if screen.height >= 3 {
            let mode = if cx.supports_images() {
                "Graphics enabled"
            } else {
                "Text fallback"
            };
            frame.render_widget(
                Paragraph::new(format!("{mode} | {}", self.status)),
                Rect::new(1, screen.height - 2, screen.width.saturating_sub(2), 1),
            );
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (96u32, 48u32);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&[
                (x * 255 / (width - 1)) as u8,
                (y * 255 / (height - 1)) as u8,
                160,
                255,
            ]);
        }
    }
    let image = Arc::new(PreparedImage::from_rgba8(width, height, &rgba)?);
    run_blocking(
        MediaApp {
            image,
            link: Link::new("Native OSC 8 link: example.com", "https://example.com").id(DOCS),
            visible: true,
            moved: false,
            overlay: false,
            status: "Unsupported terminals retain the image label.",
        },
        ThemeKind::Auto,
    )?;
    Ok(())
}
