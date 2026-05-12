use crate::gallery::{GalleryState, ThumbnailCache};
use crate::image_list::SharedImageList;
use crate::prefetch::Prefetcher;
use crate::search;
use crate::theme::Theme;
use fast_image_resize as fir;
use image::{DynamicImage, ImageReader, RgbaImage};
use ratatui::layout::Rect;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Gallery,
    Fullscreen,
}

pub struct App {
    pub images: Vec<PathBuf>,
    pub filenames: Vec<String>,
    pub mode: ViewMode,
    pub theme: Theme,
    pub cell_px: (u32, u32),

    // Gallery state
    pub gallery: GalleryState,
    pub thumb_cache: ThumbnailCache,

    // Fullscreen state
    pub current: usize,
    pub prefetcher: Prefetcher,
    pub loaded: Option<RgbaImage>,
    pub error: Option<String>,
    pub needs_render: bool,
    pub image_rect: Rect,
    pub help_visible: bool,
    loaded_for_rect: Rect,

    // Scanner state
    shared_list: SharedImageList,
    known_len: usize,
    pub scan_complete: bool,

    // Async filter
    filter_rx: Option<mpsc::Receiver<Vec<usize>>>,
}

impl App {
    pub fn new(theme: Theme, cell_px: (u32, u32), shared_list: SharedImageList) -> Self {
        Self {
            images: Vec::new(),
            filenames: Vec::new(),
            mode: ViewMode::Gallery,
            theme,
            cell_px,
            gallery: GalleryState::new(0),
            thumb_cache: ThumbnailCache::new(),
            current: 0,
            prefetcher: Prefetcher::new(),
            loaded: None,
            error: None,
            needs_render: true,
            image_rect: Rect::default(),
            help_visible: false,
            loaded_for_rect: Rect::default(),
            shared_list,
            known_len: 0,
            scan_complete: false,
            filter_rx: None,
        }
    }

    pub fn refresh_from_scanner(&mut self) {
        let new_len = self.shared_list.len();
        if new_len > self.known_len {
            let (new_paths, new_filenames) = self.shared_list.drain_since(self.known_len);
            let old_len = self.images.len();
            self.images.extend(new_paths);
            self.filenames.extend(new_filenames);

            if self.gallery.search_query.is_empty() {
                self.gallery.filtered_indices.extend(old_len..self.images.len());
            }

            self.known_len = new_len;
            if old_len == 0 {
                self.needs_render = true;
            }
        }

        if !self.scan_complete && self.shared_list.is_complete() {
            self.scan_complete = true;

            if self.images.len() == 1 && self.mode == ViewMode::Gallery {
                self.mode = ViewMode::Fullscreen;
                self.needs_render = true;
            }
        }
    }

    pub fn is_scanning(&self) -> bool {
        !self.scan_complete
    }

    pub fn update_filter(&mut self) {
        if self.gallery.search_query.is_empty() {
            self.gallery.filtered_indices = (0..self.images.len()).collect();
            self.gallery.reset_cursor();
            self.needs_render = true;
            self.filter_rx = None;
            return;
        }

        let query = self.gallery.search_query.clone();
        let filenames = self.filenames.clone();
        let (tx, rx) = mpsc::channel();
        self.filter_rx = Some(rx);

        std::thread::spawn(move || {
            let results = search::filter(&query, &filenames);
            let _ = tx.send(results);
        });
    }

    pub fn poll_filter(&mut self) {
        let results = self.filter_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(results) = results {
            self.gallery.filtered_indices = results;
            self.gallery.reset_cursor();
            self.needs_render = true;
            self.filter_rx = None;
        }
    }

    pub fn current_path(&self) -> &Path {
        &self.images[self.current]
    }

    pub fn enter_fullscreen_selected(&mut self) {
        if let Some(idx) = self.gallery.selected_index() {
            self.current = idx;
            self.mode = ViewMode::Fullscreen;
            self.loaded = None;
            self.error = None;
            self.needs_render = true;
        }
    }

    pub fn enter_gallery(&mut self) {
        self.mode = ViewMode::Gallery;
        self.loaded = None;
        self.needs_render = true;
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
        self.thumb_cache.clear();
        self.prefetcher.invalidate();
        self.needs_render = true;
    }

    pub fn load_if_needed(&mut self) -> io::Result<()> {
        if self.loaded.is_some() && self.loaded_for_rect == self.image_rect {
            return Ok(());
        }

        if let Some(img) = self.prefetcher.take(self.current, self.image_rect, self.cell_px) {
            self.loaded = Some(img);
            self.error = None;
            self.loaded_for_rect = self.image_rect;
            return Ok(());
        }

        match load_and_resize(&self.images[self.current], self.image_rect, self.cell_px) {
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

pub(crate) fn load_and_resize(path: &Path, rect: Rect, cell_px: (u32, u32)) -> io::Result<RgbaImage> {
    let img = decode_image(path)?;
    let max_w = rect.width as u32 * cell_px.0;
    let max_h = rect.height as u32 * cell_px.1;
    let (orig_w, orig_h) = (img.width(), img.height());
    if max_w == 0 || max_h == 0 {
        return Ok(img.to_rgba8());
    }
    let scale = f64::min(max_w as f64 / orig_w as f64, max_h as f64 / orig_h as f64);
    if scale >= 1.0 {
        return Ok(img.to_rgba8());
    }
    let dst_w = ((orig_w as f64 * scale) as u32).max(1);
    let dst_h = ((orig_h as f64 * scale) as u32).max(1);

    let src_rgba = img.to_rgba8();
    let src_image = fir::images::Image::from_vec_u8(orig_w, orig_h, src_rgba.into_raw(), fir::PixelType::U8x4)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    resizer
        .resize(&src_image, &mut dst_image, None)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid resized dimensions"))
}

fn decode_image(path: &Path) -> io::Result<DynamicImage> {
    #[cfg(feature = "turbo")]
    if is_jpeg(path) {
        if let Ok(img) = decode_jpeg_turbo(path) {
            return Ok(img);
        }
    }

    ImageReader::open(path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?
        .with_guessed_format()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        .decode()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(feature = "turbo")]
fn is_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
}

#[cfg(feature = "turbo")]
fn decode_jpeg_turbo(path: &Path) -> io::Result<DynamicImage> {
    let data = std::fs::read(path)?;
    let image: turbojpeg::Image<Vec<u8>> =
        turbojpeg::decompress(&data, turbojpeg::PixelFormat::RGBA)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let w = image.width as u32;
    let h = image.height as u32;
    let pixels = if image.pitch == image.width * 4 {
        image.pixels
    } else {
        image
            .pixels
            .chunks(image.pitch)
            .flat_map(|row| &row[..image.width * 4])
            .copied()
            .collect()
    };
    RgbaImage::from_raw(w, h, pixels)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid image dimensions"))
}
