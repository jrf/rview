use image::{ImageReader, RgbaImage};
use ratatui::layout::Rect;
use std::io;
use std::path::{Path, PathBuf};

pub const CELL_WIDTH: u32 = 8;
pub const CELL_HEIGHT: u32 = 16;

pub struct App {
    pub images: Vec<PathBuf>,
    pub current: usize,
    pub loaded: Option<RgbaImage>,
    pub error: Option<String>,
    pub needs_render: bool,
    pub image_rect: Rect,
    loaded_for_rect: Rect,
}

impl App {
    pub fn new(images: Vec<PathBuf>) -> Self {
        Self {
            images,
            current: 0,
            loaded: None,
            error: None,
            needs_render: true,
            image_rect: Rect::default(),
            loaded_for_rect: Rect::default(),
        }
    }

    pub fn current_path(&self) -> &Path {
        &self.images[self.current]
    }

    pub fn next(&mut self) {
        if self.current + 1 < self.images.len() {
            self.current += 1;
            self.loaded = None;
            self.error = None;
            self.needs_render = true;
        }
    }

    pub fn prev(&mut self) {
        if self.current > 0 {
            self.current -= 1;
            self.loaded = None;
            self.error = None;
            self.needs_render = true;
        }
    }

    pub fn mark_dirty(&mut self) {
        self.loaded = None;
        self.needs_render = true;
    }

    pub fn load_if_needed(&mut self) -> io::Result<()> {
        if self.loaded.is_some() && self.loaded_for_rect == self.image_rect {
            return Ok(());
        }

        match load_and_resize(&self.images[self.current], self.image_rect) {
            Ok(img) => {
                self.loaded = Some(img);
                self.error = None;
                self.loaded_for_rect = self.image_rect;
            }
            Err(e) => {
                self.loaded = None;
                self.error = Some(e.to_string());
            }
        }
        Ok(())
    }
}

fn load_and_resize(path: &Path, rect: Rect) -> io::Result<RgbaImage> {
    let img = ImageReader::open(path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?
        .decode()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let max_w = rect.width as u32 * CELL_WIDTH;
    let max_h = rect.height as u32 * CELL_HEIGHT;
    let img = img.thumbnail(max_w, max_h);

    Ok(img.to_rgba8())
}
