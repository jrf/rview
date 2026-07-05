use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, RgbaImage};
use std::io::{self, Write};

const CHUNK_SIZE: usize = 4096;

pub struct DisplayOpts {
    pub id: Option<u32>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

pub fn encode_png_to<W: Write>(out: &mut W, img: &RgbaImage, opts: &DisplayOpts) -> io::Result<()> {
    let (w, h) = img.dimensions();

    let is_ssh = std::env::var("SSH_CLIENT").is_ok() || std::env::var("SSH_CONNECTION").is_ok();

    let b64 = if is_ssh {
        let mut png_buf = Vec::new();
        PngEncoder::new_with_quality(&mut png_buf, CompressionType::Fast, FilterType::Sub)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        STANDARD.encode(&png_buf)
    } else {
        STANDARD.encode(img.as_raw())
    };

    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_SIZE).collect();
    let total = chunks.len();
    let f_val = if is_ssh { 100 } else { 32 };

    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            let id_part = opts.id.map(|id| format!(",i={id}")).unwrap_or_default();
            let cols_part = opts.cols.map(|c| format!(",c={c}")).unwrap_or_default();
            let rows_part = opts.rows.map(|r| format!(",r={r}")).unwrap_or_default();
            write!(
                out,
                "\x1b_Ga=T,q=2,f={f_val},s={w},v={h}{id_part}{cols_part}{rows_part},m={more};"
            )?;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }

    Ok(())
}

pub fn delete_all() -> io::Result<()> {
    let mut out = io::stdout().lock();
    delete_all_to(&mut out)?;
    out.flush()
}

pub fn delete_all_to<W: Write>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\")
}

/// Delete visible placements only; keep stored image data (uppercase `A` would nuke storage).
pub fn clear_placements_to<W: Write>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=a,q=2\x1b\\")
}

/// Place an already-transmitted image (by ID) at the cursor position. Cheap — no PNG payload.
pub fn place_by_id_to<W: Write>(out: &mut W, id: u32) -> io::Result<()> {
    write!(out, "\x1b_Ga=p,q=2,i={id}\x1b\\")
}
