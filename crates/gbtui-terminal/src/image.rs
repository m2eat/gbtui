//! Safe, retained terminal image rendering for the Kitty graphics protocol.
//!
//! [`PreparedImage`] decodes and transcodes input at construction time. An
//! [`ImageScene`] only transmits cached PNG/base64 bytes and never reads files,
//! spawns processes, or decodes image data while drawing.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Cursor, Write},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use base64::Engine as _;
use image::{
    ExtendedColorType, ImageEncoder, ImageError, ImageReader, Limits,
    codecs::png::{CompressionType, FilterType, PngEncoder},
    error::{ParameterError, ParameterErrorKind},
};
use ratatui::layout::Rect;

use crate::{
    brand::{MultiplexerKind, TerminalContext, TerminalName, terminal_context},
    host::HostOs,
};

const KITTY_CHUNK_SIZE: usize = 4096;
const KITTY_Z_INDEX: i32 = 1;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
static NEXT_IMAGE_NUMBER: AtomicU32 = AtomicU32::new(1);

/// A decoded image prepared for terminal rendering.
///
/// Construction validates the input with `image`'s standard decoder limits,
/// converts every supported source format to RGBA PNG, and caches the
/// base64 payload used by terminal protocols. Drawing only borrows this
/// immutable resource through an [`Arc`].
#[derive(Clone, Debug)]
pub struct PreparedImage {
    width: u32,
    height: u32,
    png: Arc<[u8]>,
    base64_png: Arc<str>,
}

impl PreparedImage {
    /// Decode PNG, JPEG, WebP, GIF, BMP, or another enabled `image` format
    /// and prepare it as a PNG for Kitty.
    ///
    /// The input is decoded with [`Limits::default`], so malformed data and
    /// decoder-limit violations are returned unchanged as [`ImageError`].
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, ImageError> {
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        reader.limits(Limits::default());
        let rgba = reader.decode()?.to_rgba8();
        Self::encode_rgba8(rgba.width(), rgba.height(), rgba.as_raw())
    }

    /// Encode an RGBA8 pixel buffer as a PNG for Kitty.
    ///
    /// `rgba` must contain exactly `width * height * 4` bytes. A malformed
    /// buffer returns an `image::ImageError`; it is never padded or truncated.
    pub fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> Result<Self, ImageError> {
        Self::encode_rgba8(width, height, rgba)
    }

    /// Image width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The validated, transcoded PNG bytes sent to Kitty.
    pub fn png_bytes(&self) -> &[u8] {
        &self.png
    }

    fn encode_rgba8(width: u32, height: u32, rgba: &[u8]) -> Result<Self, ImageError> {
        let expected_len = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0 || height == 0 || expected_len != Some(rgba.len()) {
            return Err(invalid_image_dimensions());
        }

        let mut png = Vec::new();
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
            .write_image(rgba, width, height, ExtendedColorType::Rgba8)?;
        debug_assert!(png.starts_with(PNG_SIGNATURE));

        let base64_png: Arc<str> = base64::engine::general_purpose::STANDARD
            .encode(&png)
            .into();
        Ok(Self {
            width,
            height,
            png: png.into(),
            base64_png,
        })
    }

    fn base64_png(&self) -> &str {
        &self.base64_png
    }
}

fn invalid_image_dimensions() -> ImageError {
    ImageError::Parameter(ParameterError::from_kind(
        ParameterErrorKind::DimensionMismatch,
    ))
}

/// One image placement in absolute terminal-cell coordinates.
///
/// `id` is a nonzero, stable host-assigned placement identity. `0` is
/// reserved: [`ImageScene`] ignores it so widgets can safely render their text
/// fallback before a host has assigned an ID.
#[derive(Clone, Debug)]
pub struct ImagePlacement {
    pub id: u32,
    pub image: Arc<PreparedImage>,
    pub area: Rect,
}

/// Terminal graphics protocol available to a renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GraphicsProtocol {
    /// Kitty's APC graphics protocol, also implemented by Ghostty and WezTerm.
    Kitty,
    /// iTerm2's OSC 1337 inline-image protocol.
    ///
    /// It is available only through [`render_iterm2_image`]. It is deliberately
    /// not used by [`ImageScene`], because it has no safe placement deletion
    /// primitive for a retained, dynamic frame scene.
    ITerm2,
    /// No retained graphics protocol is safe for the current terminal.
    #[default]
    None,
}

impl GraphicsProtocol {
    /// Whether this protocol supports retained dynamic frame images.
    pub const fn supports_dynamic_images(self) -> bool {
        matches!(self, Self::Kitty)
    }
}

/// Detect the process terminal's retained graphics protocol.
///
/// Unknown terminals, Windows/ConPTY, and all multiplexers conservatively use
/// [`GraphicsProtocol::None`]. iTerm2 remains text-only for dynamic frames;
/// callers that explicitly opt in to its one-shot protocol can use
/// [`render_iterm2_image`].
pub fn graphics_protocol() -> GraphicsProtocol {
    graphics_protocol_for_context(terminal_context())
}

fn graphics_protocol_for_context(context: TerminalContext) -> GraphicsProtocol {
    if context.host == HostOs::Windows || context.multiplexer != MultiplexerKind::Undetected {
        return GraphicsProtocol::None;
    }

    match context.brand {
        TerminalName::Kitty | TerminalName::Ghostty | TerminalName::WezTerm => {
            GraphicsProtocol::Kitty
        }
        TerminalName::Iterm2 => GraphicsProtocol::None,
        _ => GraphicsProtocol::None,
    }
}

/// Fit an image into a cell bounding box while preserving its pixel aspect
/// ratio for conventional terminal cells that are approximately twice as tall
/// as they are wide.
///
/// Empty source dimensions or an empty target area produce `(0, 0)`, never a
/// phantom one-cell image.
pub fn fit_image_to_cells(
    pixel_width: u32,
    pixel_height: u32,
    max_columns: u16,
    max_rows: u16,
) -> (u16, u16) {
    if pixel_width == 0 || pixel_height == 0 || max_columns == 0 || max_rows == 0 {
        return (0, 0);
    }

    // A cell's physical width:height ratio is approximately 1:2. Convert the
    // image ratio to columns-per-row before fitting it into the supplied box.
    let columns_per_row = (f64::from(pixel_width) / f64::from(pixel_height)) * 2.0;
    let rows_at_max_columns = (f64::from(max_columns) / columns_per_row).round();
    if rows_at_max_columns <= f64::from(max_rows) {
        (max_columns, rows_at_max_columns.max(1.0) as u16)
    } else {
        let columns_at_max_rows = (f64::from(max_rows) * columns_per_row).round();
        (
            columns_at_max_rows.clamp(1.0, f64::from(max_columns)) as u16,
            max_rows,
        )
    }
}

/// Build a one-shot iTerm2 OSC 1337 inline-image sequence.
///
/// This helper is intentionally separate from [`ImageScene`]. OSC 1337 has no
/// safe per-placement clear operation, so automatically placing dynamic images
/// would leave stale pixels. All terminal parameters are numeric and the image
/// body is cached base64, so source bytes cannot inject terminal controls.
pub fn render_iterm2_image(image: &PreparedImage, columns: u16, rows: u16) -> String {
    format!(
        "\x1b]1337;File=inline=1;width={columns}cells;height={rows}cells;preserveAspectRatio=1:{}\x07",
        image.base64_png()
    )
}

/// Retained Kitty graphics state for one terminal output stream.
///
/// Process-unique image numbers let the terminal allocate actual resource IDs,
/// rather than overwriting an image left by another program. A shared
/// `Arc<PreparedImage>` is uploaded once and can have multiple host placements.
#[derive(Debug)]
pub struct ImageScene {
    protocol: GraphicsProtocol,
    resources: BTreeMap<usize, ImageResource>,
    placements: BTreeMap<u32, ActivePlacement>,
}

#[derive(Debug)]
struct ImageResource {
    number: u32,
    // Retains source bytes while Kitty owns the image with this number.
    _image: Arc<PreparedImage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivePlacement {
    resource_key: usize,
    area: Rect,
}

impl ImageScene {
    /// Create retained image state for `protocol`.
    ///
    /// Only Kitty is retained. Passing `ITerm2` intentionally normalizes to
    /// `None`; use [`render_iterm2_image`] for an explicit one-shot escape.
    pub fn new(protocol: GraphicsProtocol) -> Self {
        Self {
            protocol: if protocol.supports_dynamic_images() {
                protocol
            } else {
                GraphicsProtocol::None
            },
            resources: BTreeMap::new(),
            placements: BTreeMap::new(),
        }
    }

    /// Whether this scene can render dynamic image placements.
    pub const fn supports_images(&self) -> bool {
        self.protocol.supports_dynamic_images()
    }

    /// Render the next retained placement set.
    ///
    /// Uploads occur only for a new `Arc<PreparedImage>`. Moving, replacing,
    /// hiding, and clearing placements emit precise Kitty delete operations;
    /// no delete-all operation is ever used.
    pub fn render<W: Write>(
        &mut self,
        out: &mut W,
        placements: &[ImagePlacement],
    ) -> io::Result<()> {
        if !self.supports_images() {
            return Ok(());
        }

        let mut desired = BTreeMap::new();
        for placement in placements {
            if placement.id != 0 && placement.area.width != 0 && placement.area.height != 0 {
                desired.insert(placement.id, placement);
            }
        }

        let mut control = Vec::new();
        let stale: Vec<u32> = self
            .placements
            .iter()
            .filter_map(|(&placement_id, active)| {
                let replacement = desired.get(&placement_id)?;
                (active.resource_key != resource_key(&replacement.image)
                    || active.area != replacement.area)
                    .then_some(placement_id)
            })
            .chain(
                self.placements
                    .keys()
                    .filter(|placement_id| !desired.contains_key(placement_id))
                    .copied(),
            )
            .collect();
        for placement_id in stale {
            if let Some(active) = self.placements.remove(&placement_id) {
                let number = self.resources[&active.resource_key].number;
                write_delete_placement(&mut control, number, placement_id);
            }
        }

        let mut placement_commands = Vec::new();
        for (placement_id, placement) in desired {
            if self.placements.contains_key(&placement_id) {
                continue;
            }

            let key = resource_key(&placement.image);
            let resource_number = match self.resources.get(&key) {
                Some(resource) => resource.number,
                None => {
                    let resource_number = Self::allocate_image_number()?;
                    write_upload(&mut control, resource_number, placement.image.base64_png());
                    self.resources.insert(
                        key,
                        ImageResource {
                            number: resource_number,
                            _image: Arc::clone(&placement.image),
                        },
                    );
                    resource_number
                }
            };
            write_place(
                &mut placement_commands,
                resource_number,
                placement_id,
                placement.area,
            );
            self.placements.insert(
                placement_id,
                ActivePlacement {
                    resource_key: key,
                    area: placement.area,
                },
            );
        }

        let used_resources: BTreeSet<usize> = self
            .placements
            .values()
            .map(|placement| placement.resource_key)
            .collect();
        let retired: Vec<(usize, u32)> = self
            .resources
            .iter()
            .filter_map(|(&key, resource)| {
                (!used_resources.contains(&key)).then_some((key, resource.number))
            })
            .collect();
        for (key, resource_number) in retired {
            write_delete_resource(&mut control, resource_number);
            self.resources.remove(&key);
        }

        if !placement_commands.is_empty() {
            control.extend_from_slice(b"\x1b7");
            control.extend_from_slice(&placement_commands);
            control.extend_from_slice(b"\x1b8");
        }
        if !control.is_empty() {
            out.write_all(&control)?;
        }
        Ok(())
    }

    /// Clear this scene's placements and uploaded resources from the stream.
    pub fn clear<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        self.render(out, &[])
    }

    fn allocate_image_number() -> io::Result<u32> {
        NEXT_IMAGE_NUMBER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |number| {
                number.checked_add(1)
            })
            .map_err(|_| io::Error::other("terminal image number space exhausted"))
    }
}

fn resource_key(image: &Arc<PreparedImage>) -> usize {
    Arc::as_ptr(image) as usize
}

fn write_upload(out: &mut Vec<u8>, image_number: u32, payload: &str) {
    let chunks = payload.as_bytes().chunks(KITTY_CHUNK_SIZE);
    let chunk_count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        let more = usize::from(index + 1 < chunk_count);
        out.extend_from_slice(b"\x1b_G");
        if index == 0 {
            write!(out, "a=t,f=100,t=d,q=2,I={image_number},m={more};")
                .expect("writing to a Vec cannot fail");
        } else {
            write!(out, "q=2,m={more};").expect("writing to a Vec cannot fail");
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

fn write_place(out: &mut Vec<u8>, image_number: u32, placement_id: u32, area: Rect) {
    let row = u32::from(area.y) + 1;
    let column = u32::from(area.x) + 1;
    write!(
        out,
        "\x1b[{row};{column}H\x1b_Ga=p,I={image_number},p={placement_id},c={},r={},z={KITTY_Z_INDEX},C=1,q=2\x1b\\",
        area.width, area.height,
    )
    .expect("writing to a Vec cannot fail");
}

fn write_delete_placement(out: &mut Vec<u8>, image_number: u32, placement_id: u32) {
    write!(
        out,
        "\x1b_Ga=d,d=n,I={image_number},p={placement_id},q=2\x1b\\"
    )
    .expect("writing to a Vec cannot fail");
}

fn write_delete_resource(out: &mut Vec<u8>, image_number: u32) {
    write!(out, "\x1b_Ga=d,d=N,I={image_number},q=2\x1b\\").expect("writing to a Vec cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageFormat, RgbImage};

    fn image_with_rgba(width: u32, height: u32, seed: u8) -> Arc<PreparedImage> {
        let mut rgba = vec![0; width as usize * height as usize * 4];
        let mut state = u32::from(seed).wrapping_add(1);
        for byte in &mut rgba {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (state >> 24) as u8;
        }
        Arc::new(PreparedImage::from_rgba8(width, height, &rgba).unwrap())
    }

    fn placement(
        id: u32,
        image: Arc<PreparedImage>,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> ImagePlacement {
        ImagePlacement {
            id,
            image,
            area: Rect::new(x, y, width, height),
        }
    }

    fn sequences(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
        bytes
            .split(|&byte| byte == 0x1b)
            .filter_map(|part| part.strip_prefix(b"_"))
            .filter_map(|part| {
                let separator = part.iter().position(|&byte| byte == b';')?;
                Some((&part[..separator], &part[separator + 1..]))
            })
            .collect()
    }

    fn uploaded_number(output: &[u8]) -> u32 {
        sequences(output)
            .into_iter()
            .flat_map(|(header, _)| header.split(|&byte| byte == b','))
            .find_map(|field| {
                std::str::from_utf8(field.strip_prefix(b"I=")?)
                    .ok()?
                    .parse()
                    .ok()
            })
            .expect("upload identifies its image number")
    }

    #[test]
    fn prepared_rgba_is_valid_png_with_dimensions() {
        let image = PreparedImage::from_rgba8(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        assert_eq!((image.width(), image.height()), (2, 1));
        assert!(image.png_bytes().starts_with(PNG_SIGNATURE));
        let decoded = image::load_from_memory(image.png_bytes()).unwrap();
        assert_eq!(decoded.dimensions(), (2, 1));

        let from_png = PreparedImage::from_encoded(image.png_bytes()).unwrap();
        assert_eq!((from_png.width(), from_png.height()), (2, 1));
        assert!(from_png.png_bytes().starts_with(PNG_SIGNATURE));
    }
    #[test]
    fn encoded_jpeg_is_transcoded_to_png() {
        let source = RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 255, 0]).unwrap();
        let mut jpeg = Vec::new();
        source
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();

        let image = PreparedImage::from_encoded(&jpeg).unwrap();
        assert!(image.png_bytes().starts_with(PNG_SIGNATURE));
        assert_eq!((image.width(), image.height()), (2, 1));
    }

    #[test]
    fn invalid_encoded_image_is_an_error() {
        assert!(PreparedImage::from_encoded(b"not an image").is_err());
        assert!(PreparedImage::from_rgba8(2, 2, &[0; 3]).is_err());
    }

    #[test]
    fn fit_preserves_bounds_and_empty_areas_are_empty() {
        assert_eq!(fit_image_to_cells(100, 100, 40, 20), (40, 20));
        assert_eq!(fit_image_to_cells(160, 90, 40, 20), (40, 11));
        assert_eq!(fit_image_to_cells(160, 90, 0, 20), (0, 0));
        assert_eq!(fit_image_to_cells(160, 90, 40, 0), (0, 0));
        assert_eq!(fit_image_to_cells(0, 90, 40, 20), (0, 0));
    }

    #[test]
    fn protocol_is_conservative_for_windows_and_multiplexers() {
        let kitty = TerminalContext {
            brand: TerminalName::Kitty,
            multiplexer: MultiplexerKind::Undetected,
            host: HostOs::Macos,
        };
        assert_eq!(
            graphics_protocol_for_context(kitty),
            GraphicsProtocol::Kitty
        );
        assert_eq!(
            graphics_protocol_for_context(TerminalContext {
                host: HostOs::Windows,
                ..kitty
            }),
            GraphicsProtocol::None
        );
        assert_eq!(
            graphics_protocol_for_context(TerminalContext {
                multiplexer: MultiplexerKind::Tmux,
                ..kitty
            }),
            GraphicsProtocol::None
        );
        assert_eq!(
            graphics_protocol_for_context(TerminalContext {
                brand: TerminalName::Iterm2,
                ..kitty
            }),
            GraphicsProtocol::None
        );
    }

    #[test]
    fn iterm_helper_is_explicit_and_scene_disables_it() {
        let image = image_with_rgba(1, 1, 4);
        let escape = render_iterm2_image(&image, 4, 2);
        assert!(escape.starts_with("\x1b]1337;File="));
        assert!(escape.contains("width=4cells;height=2cells"));

        let mut scene = ImageScene::new(GraphicsProtocol::ITerm2);
        let mut output = Vec::new();
        scene
            .render(&mut output, &[placement(8, image, 0, 0, 4, 2)])
            .unwrap();
        assert!(!scene.supports_images());
        assert!(output.is_empty());
    }

    #[test]
    fn upload_payload_decodes_and_is_chunked_at_4096_bytes() {
        let image = image_with_rgba(2_048, 1, 6);
        assert!(image.base64_png().len() > KITTY_CHUNK_SIZE);
        let mut scene = ImageScene::new(GraphicsProtocol::Kitty);
        let mut output = Vec::new();
        scene
            .render(&mut output, &[placement(9, Arc::clone(&image), 3, 2, 8, 1)])
            .unwrap();

        let payloads = sequences(&output);
        assert!(payloads.len() > 1);
        assert!(
            payloads
                .iter()
                .all(|(_, payload)| payload.len() <= KITTY_CHUNK_SIZE)
        );
        let mut encoded = Vec::new();
        for (_, payload) in payloads {
            encoded.extend_from_slice(payload);
        }
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            image.png_bytes()
        );
        assert!(String::from_utf8(output).unwrap().contains("f=100"));
    }

    #[test]
    fn scene_does_not_retransmit_an_unchanged_placement() {
        let image = image_with_rgba(2, 1, 8);
        let frame = [placement(11, Arc::clone(&image), 1, 2, 4, 1)];
        let mut scene = ImageScene::new(GraphicsProtocol::Kitty);
        let mut first = Vec::new();
        scene.render(&mut first, &frame).unwrap();
        assert!(String::from_utf8(first).unwrap().contains("a=t"));

        let mut second = Vec::new();
        scene.render(&mut second, &frame).unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn scene_replaces_moves_hides_and_releases_precisely() {
        let first_image = image_with_rgba(2, 1, 10);
        let replacement = image_with_rgba(2, 1, 20);
        let mut scene = ImageScene::new(GraphicsProtocol::Kitty);
        let mut output = Vec::new();
        scene
            .render(
                &mut output,
                &[placement(17, Arc::clone(&first_image), 1, 1, 4, 1)],
            )
            .unwrap();
        let original_number = uploaded_number(&output);

        output.clear();
        scene
            .render(
                &mut output,
                &[placement(17, Arc::clone(&replacement), 1, 1, 4, 1)],
            )
            .unwrap();
        let replacement_number = uploaded_number(&output);
        let replaced = String::from_utf8(output.clone()).unwrap();
        assert!(replaced.contains(&format!("a=d,d=n,I={original_number},p=17")));
        assert!(replaced.contains(&format!("a=d,d=N,I={original_number}")));

        output.clear();
        scene
            .render(
                &mut output,
                &[placement(17, Arc::clone(&replacement), 5, 3, 4, 1)],
            )
            .unwrap();
        let moved = String::from_utf8(output.clone()).unwrap();
        assert!(moved.contains(&format!("a=d,d=n,I={replacement_number},p=17")));
        assert!(!moved.contains("a=t,"));
        assert!(moved.contains("\x1b[4;6H"));

        output.clear();
        scene.clear(&mut output).unwrap();
        let cleared = String::from_utf8(output).unwrap();
        assert!(cleared.contains(&format!("a=d,d=n,I={replacement_number},p=17")));
        assert!(cleared.contains(&format!("a=d,d=N,I={replacement_number}")));
    }

    #[test]
    fn scene_reuses_one_resource_across_distinct_placement_ids() {
        let image = image_with_rgba(2, 1, 30);
        let mut scene = ImageScene::new(GraphicsProtocol::Kitty);
        let mut output = Vec::new();
        scene
            .render(
                &mut output,
                &[
                    placement(101, Arc::clone(&image), 0, 0, 4, 1),
                    placement(202, Arc::clone(&image), 8, 0, 4, 1),
                ],
            )
            .unwrap();
        let number = uploaded_number(&output);
        let rendered = String::from_utf8(output.clone()).unwrap();
        assert_eq!(rendered.matches("a=t,f=100").count(), 1);
        assert!(rendered.contains("p=101"));
        assert!(rendered.contains("p=202"));
        assert!(rendered.contains(&format!("I={number},p=101")));
        assert!(rendered.contains(&format!("I={number},p=202")));
        output.clear();
        scene
            .render(&mut output, &[placement(202, image, 8, 0, 4, 1)])
            .unwrap();
        let removed = String::from_utf8(output).unwrap();
        assert!(removed.contains(&format!("a=d,d=n,I={number},p=101")));
        assert!(!removed.contains("p=202"));
        assert!(!removed.contains("d=N"));
        assert!(!removed.contains("a=t"));
    }

    #[test]
    fn separate_scenes_do_not_alias_resource_numbers() {
        let frame = [placement(7, image_with_rgba(1, 1, 31), 0, 0, 1, 1)];
        let mut first = ImageScene::new(GraphicsProtocol::Kitty);
        let mut second = ImageScene::new(GraphicsProtocol::Kitty);
        let mut first_output = Vec::new();
        let mut second_output = Vec::new();
        first.render(&mut first_output, &frame).unwrap();
        second.render(&mut second_output, &frame).unwrap();
        assert_ne!(
            uploaded_number(&first_output),
            uploaded_number(&second_output)
        );
        first_output.clear();
        first.clear(&mut first_output).unwrap();
        let cleared = String::from_utf8(first_output).unwrap();
        assert!(!cleared.contains(&format!("I={},", uploaded_number(&second_output))));
    }

    #[test]
    fn none_protocol_emits_no_graphics_bytes() {
        let mut scene = ImageScene::new(GraphicsProtocol::None);
        let mut output = Vec::new();
        scene
            .render(
                &mut output,
                &[placement(1, image_with_rgba(1, 1, 40), 0, 0, 1, 1)],
            )
            .unwrap();
        scene.clear(&mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn zero_area_placement_is_fallback_only() {
        let mut scene = ImageScene::new(GraphicsProtocol::Kitty);
        let mut output = Vec::new();
        scene
            .render(
                &mut output,
                &[placement(3, image_with_rgba(1, 1, 50), 0, 0, 0, 1)],
            )
            .unwrap();
        assert!(output.is_empty());
    }
}
