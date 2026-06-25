use crate::gallery::{GalleryState, ThumbnailCache};
use crate::image_list::SharedImageList;
use crate::prefetch::Prefetcher;
use crate::search;
use crate::theme::Theme;
use fast_image_resize as fir;
use image::{DynamicImage, ImageReader, RgbaImage};
use ratatui::layout::Rect;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Gallery,
    Fullscreen,
    #[cfg(feature = "video")]
    Video,
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

    // Async fullscreen decode
    fullscreen_rx: Option<mpsc::Receiver<io::Result<RgbaImage>>>,
    fullscreen_target: Option<(usize, Rect)>,

    #[cfg(feature = "video")]
    pub video: Option<crate::video::VideoPlayback>,

    // Scanner state
    shared_list: SharedImageList,
    known_len: usize,
    pub scan_complete: bool,

    // Async filter
    filter_rx: Option<mpsc::Receiver<Vec<usize>>>,

    // Async thumbnails
    thumb_tx: mpsc::Sender<(u32, usize, RgbaImage)>,
    thumb_rx: mpsc::Receiver<(u32, usize, RgbaImage)>,
    thumb_loading: HashSet<usize>,
    thumb_generation: u32,
}

impl App {
    pub fn new(theme: Theme, cell_px: (u32, u32), shared_list: SharedImageList) -> Self {
        let (thumb_tx, thumb_rx) = mpsc::channel();
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
            fullscreen_rx: None,
            fullscreen_target: None,
            #[cfg(feature = "video")]
            video: None,
            shared_list,
            known_len: 0,
            scan_complete: false,
            filter_rx: None,
            thumb_tx,
            thumb_rx,
            thumb_loading: HashSet::new(),
            thumb_generation: 0,
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
                self.gallery
                    .filtered_indices
                    .extend(old_len..self.images.len());
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
            #[cfg(feature = "video")]
            if crate::video::is_video(&self.images[idx]) {
                self.enter_video(idx);
                return;
            }
            self.current = idx;
            self.mode = ViewMode::Fullscreen;
            self.error = None;
            self.needs_render = true;
            self.fullscreen_rx = None;
            self.fullscreen_target = None;
            self.loaded = None;
        }
    }

    #[cfg(feature = "video")]
    pub fn enter_video(&mut self, idx: usize) {
        self.current = idx;
        self.mode = ViewMode::Video;
        self.error = None;
        self.needs_render = true;
        self.loaded = None;
        self.video = None;
    }

    #[cfg(feature = "video")]
    pub fn open_video_if_needed(&mut self) {
        if self.video.is_some() {
            return;
        }
        let path = &self.images[self.current];
        match crate::video::VideoPlayback::open(path, self.image_rect, self.cell_px) {
            Ok(v) => self.video = Some(v),
            Err(e) => {
                self.error = Some(e.to_string());
                self.mode = ViewMode::Fullscreen;
            }
        }
    }

    #[cfg(feature = "video")]
    pub fn exit_video(&mut self) {
        if let Some(mut v) = self.video.take() {
            v.stop();
        }
        self.mode = ViewMode::Gallery;
        self.needs_render = true;
    }

    fn start_fullscreen_decode(&mut self, idx: usize, rect: Rect) {
        let (tx, rx) = mpsc::channel();
        self.fullscreen_rx = Some(rx);
        self.fullscreen_target = Some((idx, rect));
        let path = self.images[idx].clone();
        let cell_px = self.cell_px;
        rayon::spawn(move || {
            let result = load_and_resize(&path, rect, cell_px);
            let _ = tx.send(result);
        });
    }

    pub fn poll_fullscreen(&mut self) -> bool {
        let result = self
            .fullscreen_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            if let Some((idx, rect)) = self.fullscreen_target {
                if idx == self.current
                    && rect == self.image_rect
                    && self.mode == ViewMode::Fullscreen
                {
                    match result {
                        Ok(img) => {
                            self.loaded = Some(img);
                            self.error = None;
                            self.loaded_for_rect = rect;
                            self.needs_render = true;
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                        }
                    }
                }
            }
            self.fullscreen_rx = None;
            self.fullscreen_target = None;
            return true;
        }
        false
    }

    pub fn enter_gallery(&mut self) {
        self.mode = ViewMode::Gallery;
        self.loaded = None;
        self.needs_render = true;
    }

    pub fn next(&mut self) {
        if self.current + 1 < self.images.len() {
            self.jump_to(self.current + 1);
        }
    }

    pub fn prev(&mut self) {
        if self.current > 0 {
            self.jump_to(self.current - 1);
        }
    }

    pub fn first(&mut self) {
        if !self.images.is_empty() && self.current != 0 {
            self.jump_to(0);
        }
    }

    pub fn last(&mut self) {
        let last = self.images.len().saturating_sub(1);
        if !self.images.is_empty() && self.current != last {
            self.jump_to(last);
        }
    }

    fn jump_to(&mut self, target: usize) {
        #[cfg(feature = "video")]
        if let Some(mut v) = self.video.take() {
            v.stop();
        }

        self.current = target;
        self.error = None;
        self.needs_render = true;
        self.fullscreen_rx = None;
        self.fullscreen_target = None;

        #[cfg(feature = "video")]
        if crate::video::is_video(&self.images[self.current]) {
            self.mode = ViewMode::Video;
            self.loaded = None;
            return;
        }

        self.mode = ViewMode::Fullscreen;
        if let Some(img) =
            self.prefetcher
                .take_resized(self.current, self.image_rect, self.cell_px)
        {
            self.loaded = Some(img);
            self.loaded_for_rect = self.image_rect;
        } else {
            self.loaded = None;
            self.start_fullscreen_decode(self.current, self.image_rect);
        }
    }

    pub fn poll_thumbnails(&mut self) -> Vec<usize> {
        let mut new_indices = Vec::new();
        while let Ok((generation, img_idx, img)) = self.thumb_rx.try_recv() {
            self.thumb_loading.remove(&img_idx);
            if generation == self.thumb_generation {
                self.thumb_cache.insert(img_idx, img);
                new_indices.push(img_idx);
            }
        }
        new_indices
    }

    pub fn spawn_thumb_decode(&mut self, img_idx: usize, path: PathBuf, rect: Rect) {
        if self.thumb_cache.contains(img_idx) || self.thumb_loading.contains(&img_idx) {
            return;
        }
        self.thumb_loading.insert(img_idx);
        let tx = self.thumb_tx.clone();
        let cell_px = self.cell_px;
        let generation = self.thumb_generation;
        rayon::spawn(move || {
            let target_w = rect.width as u32 * cell_px.0;
            let target_h = rect.height as u32 * cell_px.1;

            if let Some(img) = try_load_from_disk_cache(&path, target_w, target_h) {
                let _ = tx.send((generation, img_idx, img));
                return;
            }

            #[cfg(feature = "video")]
            let res = if crate::video::is_video(&path) {
                crate::video::decode_first_frame(&path, rect, cell_px)
            } else {
                load_and_resize(&path, rect, cell_px)
            };

            #[cfg(not(feature = "video"))]
            let res = load_and_resize(&path, rect, cell_px);

            if let Ok(img) = res {
                try_save_to_disk_cache(&path, target_w, target_h, &img);
                let _ = tx.send((generation, img_idx, img));
            }
        });
    }

    pub fn pre_decode_hovered(&mut self) {
        if let Some(idx) = self.gallery.selected_index() {
            self.prefetcher.kick_gallery(idx, &self.images);
        }
    }

    pub fn mark_dirty(&mut self) {
        self.loaded = None;
        self.thumb_cache.clear();
        self.thumb_loading.clear();
        self.thumb_generation += 1;
        self.prefetcher.invalidate();
        self.needs_render = true;
    }

    pub fn load_if_needed(&mut self) {
        if self.loaded.is_some() && self.loaded_for_rect == self.image_rect {
            return;
        }

        if let Some(img) = self
            .prefetcher
            .take_resized(self.current, self.image_rect, self.cell_px)
        {
            self.loaded = Some(img);
            self.error = None;
            self.loaded_for_rect = self.image_rect;
            return;
        }

        if let Some((_, rect)) = self.fullscreen_target {
            if rect == self.image_rect {
                return;
            }
        }

        self.start_fullscreen_decode(self.current, self.image_rect);
    }
}

pub(crate) fn load_and_resize(
    path: &Path,
    rect: Rect,
    cell_px: (u32, u32),
) -> io::Result<RgbaImage> {
    let target_w = rect.width as u32 * cell_px.0;
    let target_h = rect.height as u32 * cell_px.1;
    let hint = if target_w > 0 && target_h > 0 {
        Some((target_w, target_h))
    } else {
        None
    };
    let img = decode_image_with_hint(path, hint)?;
    Ok(resize_decoded(&img, rect, cell_px))
}

pub(crate) fn resize_decoded_to_dims(img: &DynamicImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (orig_w, orig_h) = (img.width(), img.height());
    if max_w == 0 || max_h == 0 || orig_w == 0 || orig_h == 0 {
        return img.to_rgba8();
    }
    let scale = f64::min(max_w as f64 / orig_w as f64, max_h as f64 / orig_h as f64);
    if scale >= 1.0 {
        return img.to_rgba8();
    }
    let dst_w = ((orig_w as f64 * scale) as u32).max(1);
    let dst_h = ((orig_h as f64 * scale) as u32).max(1);

    let src_rgba = img.to_rgba8();
    let Ok(src_image) =
        fir::images::Image::from_vec_u8(orig_w, orig_h, src_rgba.into_raw(), fir::PixelType::U8x4)
    else {
        return img.to_rgba8();
    };
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    if resizer.resize(&src_image, &mut dst_image, None).is_err() {
        return img.to_rgba8();
    }

    RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec()).unwrap_or_else(|| img.to_rgba8())
}

pub(crate) fn resize_decoded(img: &DynamicImage, rect: Rect, cell_px: (u32, u32)) -> RgbaImage {
    let max_w = rect.width as u32 * cell_px.0;
    let max_h = rect.height as u32 * cell_px.1;
    resize_decoded_to_dims(img, max_w, max_h)
}

pub(crate) fn decode_image_with_hint(
    path: &Path,
    target: Option<(u32, u32)>,
) -> io::Result<DynamicImage> {
    #[cfg(feature = "turbo")]
    if is_jpeg(path) {
        if let Ok(img) = decode_jpeg_turbo(path, target) {
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
fn pick_scaling_factor(
    orig_w: usize,
    orig_h: usize,
    target_w: u32,
    target_h: u32,
) -> turbojpeg::ScalingFactor {
    let candidates = [
        turbojpeg::ScalingFactor::ONE_EIGHTH,
        turbojpeg::ScalingFactor::ONE_QUARTER,
        turbojpeg::ScalingFactor::ONE_HALF,
        turbojpeg::ScalingFactor::ONE,
    ];
    let tw = target_w as usize;
    let th = target_h as usize;
    for &sf in &candidates {
        let sw = sf.scale(orig_w);
        let sh = sf.scale(orig_h);
        if sw >= tw && sh >= th {
            return sf;
        }
    }
    turbojpeg::ScalingFactor::ONE
}

#[cfg(feature = "turbo")]
fn decode_jpeg_turbo(path: &Path, target: Option<(u32, u32)>) -> io::Result<DynamicImage> {
    let data = std::fs::read(path)?;
    let mut decompressor = turbojpeg::Decompressor::new()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let header = decompressor
        .read_header(&data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let scaling = match target {
        Some((tw, th)) if tw > 0 && th > 0 && !header.is_lossless => {
            pick_scaling_factor(header.width, header.height, tw, th)
        }
        _ => turbojpeg::ScalingFactor::ONE,
    };

    if scaling != turbojpeg::ScalingFactor::ONE {
        decompressor
            .set_scaling_factor(scaling)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    }

    let scaled = header.scaled(scaling);
    let pitch = scaled.width * 4;
    let mut image = turbojpeg::Image {
        pixels: vec![0u8; scaled.height * pitch],
        width: scaled.width,
        pitch,
        height: scaled.height,
        format: turbojpeg::PixelFormat::RGBA,
    };

    decompressor
        .decompress(&data, image.as_deref_mut())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let w = image.width as u32;
    let h = image.height as u32;
    RgbaImage::from_raw(w, h, image.pixels)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid image dimensions"))
}

fn get_cache_file_path(path: &Path, target_w: u32, target_h: u32) -> Option<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::fs;
    use std::hash::{Hash, Hasher};

    let metadata = fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    target_w.hash(&mut hasher);
    target_h.hash(&mut hasher);
    let filename = format!("{:016x}.png", hasher.finish());

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let mut cache_dir = PathBuf::from(home);
    cache_dir.push(".cache");
    cache_dir.push("rview");

    Some(cache_dir.join(filename))
}

fn try_load_from_disk_cache(path: &Path, target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let cache_path = get_cache_file_path(path, target_w, target_h)?;
    if cache_path.exists() {
        image::open(&cache_path).ok().map(|img| img.to_rgba8())
    } else {
        None
    }
}

fn try_save_to_disk_cache(
    path: &Path,
    target_w: u32,
    target_h: u32,
    img: &RgbaImage,
) -> Option<()> {
    use std::fs;
    let cache_path = get_cache_file_path(path, target_w, target_h)?;
    let parent = cache_path.parent()?;
    fs::create_dir_all(parent).ok()?;
    img.save(&cache_path).ok()?;
    Some(())
}
