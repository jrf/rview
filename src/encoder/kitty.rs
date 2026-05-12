use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::RgbaImage;
use std::io::{self, Write};

const CHUNK_SIZE: usize = 4096;

pub fn encode_to<W: Write>(out: &mut W, img: &RgbaImage) -> io::Result<()> {
    let (w, h) = img.dimensions();
    let raw = img.as_raw();
    let b64 = STANDARD.encode(raw);

    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_SIZE).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            write!(out, "\x1b_Ga=T,f=32,s={w},v={h},m={more};")?;
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
    out.write_all(b"\x1b_Ga=d,d=A\x1b\\")
}
