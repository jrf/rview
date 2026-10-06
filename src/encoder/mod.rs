pub mod kitty;

use image::RgbaImage;
use std::io::{self, Write};

/// Fullscreen double-buffer slots, separate from gallery's index-based image IDs.
pub const FULLSCREEN_IMAGE_IDS: [u32; 2] = [u32::MAX - 1, u32::MAX];

pub struct DisplayOptions {
    pub id: Option<u32>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

pub trait GraphicsBackend {
    fn transmit(
        &self,
        out: &mut dyn Write,
        image: &RgbaImage,
        options: &DisplayOptions,
    ) -> io::Result<()>;
    fn upload_to(&self, out: &mut dyn Write, image: &RgbaImage, id: u32) -> io::Result<()>;
    fn delete_image_to(&self, out: &mut dyn Write, id: u32) -> io::Result<()>;
    fn place_with_id_to(
        &self,
        out: &mut dyn Write,
        image_id: u32,
        placement_id: u32,
    ) -> io::Result<()>;
    fn delete_all(&self) -> io::Result<()>;
    fn delete_all_to(&self, out: &mut dyn Write) -> io::Result<()>;
    fn clear_placements_to(&self, out: &mut dyn Write) -> io::Result<()>;
    fn place_by_id_to(&self, out: &mut dyn Write, id: u32) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct KittyBackend;

impl GraphicsBackend for KittyBackend {
    fn transmit(
        &self,
        out: &mut dyn Write,
        image: &RgbaImage,
        options: &DisplayOptions,
    ) -> io::Result<()> {
        kitty::encode_png_to(out, image, options)
    }

    fn upload_to(&self, out: &mut dyn Write, image: &RgbaImage, id: u32) -> io::Result<()> {
        kitty::upload_to(out, image, id)
    }

    fn delete_image_to(&self, out: &mut dyn Write, id: u32) -> io::Result<()> {
        kitty::delete_image_to(out, id)
    }

    fn place_with_id_to(
        &self,
        out: &mut dyn Write,
        image_id: u32,
        placement_id: u32,
    ) -> io::Result<()> {
        kitty::place_with_id_to(out, image_id, placement_id)
    }

    fn delete_all(&self) -> io::Result<()> {
        kitty::delete_all()
    }

    fn delete_all_to(&self, out: &mut dyn Write) -> io::Result<()> {
        kitty::delete_all_to(out)
    }

    fn clear_placements_to(&self, out: &mut dyn Write) -> io::Result<()> {
        kitty::clear_placements_to(out)
    }

    fn place_by_id_to(&self, out: &mut dyn Write, id: u32) -> io::Result<()> {
        kitty::place_by_id_to(out, id)
    }
}
