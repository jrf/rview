use super::DisplayOptions;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, RgbaImage};
use std::io::{self, Write};

const CHUNK_SIZE: usize = 4096;
const RAW_CHUNK_SIZE: usize = CHUNK_SIZE / 4 * 3;

pub fn encode_png_to<W: Write + ?Sized>(
    out: &mut W,
    img: &RgbaImage,
    opts: &DisplayOptions,
) -> io::Result<()> {
    encode_image_to(out, img, opts, 'T')
}

/// Upload image data without creating a placement or disturbing another image ID.
pub fn upload_to<W: Write + ?Sized>(out: &mut W, img: &RgbaImage, id: u32) -> io::Result<()> {
    encode_image_to(
        out,
        img,
        &DisplayOptions {
            id: Some(id),
            cols: None,
            rows: None,
        },
        't',
    )
}

fn encode_image_to<W: Write + ?Sized>(
    out: &mut W,
    img: &RgbaImage,
    opts: &DisplayOptions,
    action: char,
) -> io::Result<()> {
    let (w, h) = img.dimensions();
    // Fullscreen uploads favor the measured lower wire cost even locally.
    // Gallery/video transmit keeps the existing SSH-only PNG selection.
    let use_png = action == 't'
        || std::env::var("SSH_CLIENT").is_ok()
        || std::env::var("SSH_CONNECTION").is_ok();
    let png = if use_png {
        let mut png_buf = Vec::new();
        PngEncoder::new_with_quality(&mut png_buf, CompressionType::Fast, FilterType::Sub)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Some(png_buf)
    } else {
        None
    };
    let data = png.as_deref().unwrap_or_else(|| img.as_raw());
    let f_val = if use_png { 100 } else { 32 };
    let mut encoded = [0; CHUNK_SIZE];
    let mut chunks = data.chunks(RAW_CHUNK_SIZE).peekable();

    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            write!(out, "\x1b_Ga={action},q=2,C=1,f={f_val},s={w},v={h}")?;
            if let Some(id) = opts.id {
                write!(out, ",i={id}")?;
            }
            if let Some(cols) = opts.cols {
                write!(out, ",c={cols}")?;
            }
            if let Some(rows) = opts.rows {
                write!(out, ",r={rows}")?;
            }
            write!(out, ",m={more};")?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        let len = STANDARD
            .encode_slice(chunk, &mut encoded)
            .expect("base64 chunk fits the fixed output buffer");
        out.write_all(&encoded[..len])?;
        out.write_all(b"\x1b\\")?;
    }

    Ok(())
}

pub fn delete_all() -> io::Result<()> {
    let mut out = io::stdout().lock();
    delete_all_to(&mut out)?;
    out.flush()
}

pub fn delete_all_to<W: Write + ?Sized>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\")
}

/// Delete visible placements only; keep stored image data (uppercase `A` would nuke storage).
pub fn clear_placements_to<W: Write + ?Sized>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=a,q=2\x1b\\")
}

/// Place an already-transmitted image (by ID) at the cursor position. Cheap — no PNG payload.
/// `C=1` keeps the text cursor fixed so large placements never scroll the grid.
pub fn place_by_id_to<W: Write + ?Sized>(out: &mut W, id: u32) -> io::Result<()> {
    write!(out, "\x1b_Ga=p,q=2,C=1,i={id}\x1b\\")
}

/// A stable placement ID replaces an earlier placement instead of accumulating copies.
pub fn place_with_id_to<W: Write + ?Sized>(
    out: &mut W,
    image_id: u32,
    placement_id: u32,
) -> io::Result<()> {
    write!(out, "\x1b_Ga=p,q=2,C=1,i={image_id},p={placement_id}\x1b\\")
}

/// Retire all placements and stored data for this image, including unplaced uploads.
pub fn delete_image_to<W: Write + ?Sized>(out: &mut W, id: u32) -> io::Result<()> {
    write!(out, "\x1b_Ga=d,d=I,q=2,i={id}\x1b\\")
}

#[cfg(test)]
mod tests {
    use super::{clear_placements_to, delete_all_to, encode_png_to, place_by_id_to, upload_to};
    use crate::encoder::DisplayOptions;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use image::RgbaImage;

    #[test]
    fn management_sequences_match_the_kitty_protocol() {
        let mut output = Vec::new();
        delete_all_to(&mut output).unwrap();
        clear_placements_to(&mut output).unwrap();
        place_by_id_to(&mut output, 42).unwrap();
        assert_eq!(
            output,
            b"\x1b_Ga=d,d=A,q=2\x1b\\\x1b_Ga=d,d=a,q=2\x1b\\\x1b_Ga=p,q=2,C=1,i=42\x1b\\"
        );
    }

    #[test]
    fn transmission_includes_dimensions_and_id() {
        let image = RgbaImage::new(2, 3);
        let mut output = Vec::new();
        encode_png_to(
            &mut output,
            &image,
            &DisplayOptions {
                id: Some(7),
                cols: None,
                rows: None,
            },
        )
        .unwrap();
        let encoded = String::from_utf8(output).unwrap();
        assert!(encoded.starts_with("\u{1b}_Ga=T,q=2,"));
        assert!(encoded.contains(",C=1,"));
        assert!(encoded.contains(",s=2,v=3,i=7,"));
        assert!(encoded.ends_with("\u{1b}\\"));
    }

    #[test]
    fn upload_chunks_round_trip_without_display_or_interleaved_commands() {
        let mut image = RgbaImage::new(128, 128);
        let mut random = 1u32;
        for byte in image.as_mut() {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *byte = random as u8;
        }
        let mut output = Vec::new();
        upload_to(&mut output, &image, 42).unwrap();
        let output = String::from_utf8(output).unwrap();
        let commands: Vec<_> = output
            .split("\x1b\\")
            .filter(|chunk| !chunk.is_empty())
            .collect();
        assert!(commands.len() > 1);
        let mut payload = String::new();
        for (index, command) in commands.iter().enumerate() {
            let (header, chunk) = command
                .strip_prefix("\x1b_G")
                .unwrap()
                .split_once(';')
                .unwrap();
            let more = usize::from(index + 1 < commands.len());
            if index == 0 {
                assert!(header.starts_with("a=t,q=2,"));
                assert!(header.contains(",i=42,"));
                assert!(header.contains(",f=100,"));
                assert!(header.ends_with(&format!("m={more}")));
            } else {
                assert_eq!(header, format!("m={more}"));
            }
            assert!(chunk.len() <= 4096);
            assert_eq!(chunk.len() % 4, 0);
            payload.push_str(chunk);
        }
        let data = STANDARD.decode(payload).unwrap();
        assert_eq!(image::load_from_memory(&data).unwrap().to_rgba8(), image);
    }
}
