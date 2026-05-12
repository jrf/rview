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
    let mut png_buf = Vec::new();
    PngEncoder::new_with_quality(&mut png_buf, CompressionType::Fast, FilterType::Sub)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let b64 = STANDARD.encode(&png_buf);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_SIZE).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            let id_part = opts.id.map(|id| format!(",i={id}")).unwrap_or_default();
            let cols_part = opts.cols.map(|c| format!(",c={c}")).unwrap_or_default();
            let rows_part = opts.rows.map(|r| format!(",r={r}")).unwrap_or_default();
            write!(out, "\x1b_Ga=T,q=2,f=100,s={w},v={h}{id_part}{cols_part}{rows_part},m={more};")?;
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
