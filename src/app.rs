use crate::encoder::{GraphicsBackend, KittyBackend};
use crate::gallery::{GalleryState, ThumbnailCache};
use crate::image_list::SharedImageList;
use crate::picker::{self, PickerState};
use crate::prefetch::Prefetcher;
use crate::scanner;
use crate::search;
use crate::theme::{NamedTheme, Theme};
use directories::ProjectDirs;
use fast_image_resize as fir;
use image::{DynamicImage, ImageReader, RgbaImage};
use ratatui::layout::Rect;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, mpsc};

/// Multiplier applied on each zoom step.
const ZOOM_STEP: f64 = 1.25;
/// Maximum zoom factor (relative to fit-to-window).
const MAX_ZOOM: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Gallery,
    Fullscreen,
    Picker,
    #[cfg(feature = "video")]
    Video,
}

pub struct PendingDelete {
    pub paths: Vec<PathBuf>,
    pub from_fullscreen: bool,
    pub permanent: bool,
}

pub struct ThemePickerState {
    pub selected: usize,
    pub original_index: usize,
    pub scroll: usize,
    pub visible_height: usize,
}

enum ZoomSource {
    Raster(DynamicImage),
    Svg(resvg::usvg::Tree),
}

impl ZoomSource {
    fn dimensions(&self) -> (f64, f64) {
        match self {
            Self::Raster(image) => (f64::from(image.width()), f64::from(image.height())),
            Self::Svg(tree) => (
                f64::from(tree.size().width()),
                f64::from(tree.size().height()),
            ),
        }
    }

    /// Source crop geometry, shared by zoom rendering and pointer-relative panning.
    fn crop(&self, zoom: f64, vw: u32, vh: u32) -> ZoomCrop {
        let (ow, oh) = self.dimensions();
        let fit = f64::min(f64::from(vw) / ow, f64::from(vh) / oh);
        let scale = fit.min(1.0) * zoom;
        let (width, height) = match self {
            Self::Raster(_) => (
                (f64::from(vw) / scale).round().clamp(1.0, ow),
                (f64::from(vh) / scale).round().clamp(1.0, oh),
            ),
            Self::Svg(_) => (
                (f64::from(vw) / scale).min(ow),
                (f64::from(vh) / scale).min(oh),
            ),
        };
        ZoomCrop {
            width,
            height,
            max_x: ow - width,
            max_y: oh - height,
            scale,
        }
    }
}

struct ZoomCrop {
    width: f64,
    height: f64,
    max_x: f64,
    max_y: f64,
    scale: f64,
}

struct ZoomRequest {
    source: Arc<ZoomSource>,
    index: usize,
    rect: Rect,
    cell_px: (u32, u32),
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

struct SvgZoomCache {
    transform: resvg::tiny_skia::Transform,
    image: RgbaImage,
}

struct ZoomResult {
    request: ZoomRequest,
    cache: Option<SvgZoomCache>,
    image: io::Result<RgbaImage>,
}

impl ZoomRequest {
    fn matches(&self, app: &App) -> bool {
        self.index == app.current
            && self.rect == app.image_rect
            && self.cell_px == app.cell_px
            && self.zoom == app.zoom
    }

    fn viewport(&self) -> io::Result<(ZoomCrop, u32, u32)> {
        let (vw, vh) = (
            (u32::from(self.rect.width) * self.cell_px.0).max(1),
            (u32::from(self.rect.height) * self.cell_px.1).max(1),
        );
        let (ow, oh) = self.source.dimensions();
        if ow == 0.0 || oh == 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid image dimensions",
            ));
        }
        let crop = self.source.crop(self.zoom, vw, vh);
        let dst_w = ((crop.width * crop.scale).round() as u32).max(1).min(vw);
        let dst_h = ((crop.height * crop.scale).round() as u32).max(1).min(vh);
        Ok((crop, dst_w, dst_h))
    }

    fn cached_image(&self, cache: &SvgZoomCache) -> io::Result<Option<RgbaImage>> {
        let ZoomSource::Svg(tree) = self.source.as_ref() else {
            return Ok(None);
        };
        let (crop, width, height) = self.viewport()?;
        let view = SvgViewport::new(
            tree,
            width,
            height,
            crop.scale,
            (
                self.pan_x * crop.max_x * crop.scale,
                self.pan_y * crop.max_y * crop.scale,
            ),
        )?;
        Ok(view.cached_image(cache))
    }

    fn render(&self, cache: &mut Option<SvgZoomCache>) -> io::Result<RgbaImage> {
        let (crop, dst_w, dst_h) = self.viewport()?;
        let x = self.pan_x * crop.max_x;
        let y = self.pan_y * crop.max_y;
        match self.source.as_ref() {
            ZoomSource::Raster(image) => {
                let cropped = image.crop_imm(
                    x.round() as u32,
                    y.round() as u32,
                    crop.width as u32,
                    crop.height as u32,
                );
                Ok(resize_to_exact(&cropped, dst_w, dst_h))
            }
            ZoomSource::Svg(tree) => render_svg_cached(
                tree,
                dst_w,
                dst_h,
                crop.scale,
                (x * crop.scale, y * crop.scale),
                cache,
            ),
        }
    }
}

pub struct App {
    pub images: Vec<PathBuf>,
    pub filenames: Vec<String>,
    pub mode: ViewMode,
    pub theme: Theme,
    pub themes: Vec<NamedTheme>,
    pub theme_index: usize,
    pub theme_picker: Option<ThemePickerState>,
    pub cell_px: (u32, u32),
    pub graphics: KittyBackend,

    // Gallery state
    pub gallery: GalleryState,
    pub thumb_cache: ThumbnailCache,
    pub selection: HashSet<PathBuf>,
    pub pending_delete: Option<PendingDelete>,
    pub delete_error: Option<String>,

    // Directory picker state
    pub picker: Option<PickerState>,
    pub current_dir: Option<PathBuf>,
    pub initial_dir: Option<PathBuf>,

    // Fullscreen state
    pub current: usize,
    pub prefetcher: Prefetcher,
    pub loaded: Option<RgbaImage>,
    pub loaded_revision: u64,
    pub error: Option<String>,
    pub needs_render: bool,
    pub image_rect: Rect,
    pub help_visible: bool,
    loaded_for_rect: Rect,

    // Async fullscreen decode
    fullscreen_rx: Option<mpsc::Receiver<io::Result<RgbaImage>>>,
    fullscreen_target: Option<(usize, Rect)>,

    // Fullscreen zoom / pan
    pub zoom: f64,
    pan_x: f64,
    pan_y: f64,
    zoom_dirty: bool,
    /// Original raster pixels or a parsed SVG retained for rendering zoomed views.
    source: Option<Arc<ZoomSource>>,
    source_for: Option<usize>,
    source_rx: Option<mpsc::Receiver<io::Result<ZoomSource>>>,
    source_target: Option<usize>,
    // One render in flight; input updates replace desired geometry, never queue jobs.
    zoom_rx: Option<mpsc::Receiver<ZoomResult>>,
    zoom_cache: Option<SvgZoomCache>,

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

    // Kitty IDs currently transmitted to the terminal — safe to `place_by_id` instead of retransmit.
    pub transmitted_image_ids: HashSet<u32>,
    /// True when the LRU thumb cache was invalidated but backend storage has not been cleared yet.
    /// Consumed by render_gallery_images to issue a full `d=A` at the next redraw.
    pub graphics_storage_dirty: bool,
}

impl App {
    pub fn new(theme: Theme, cell_px: (u32, u32), shared_list: SharedImageList) -> Self {
        let (thumb_tx, thumb_rx) = mpsc::channel();
        Self {
            images: Vec::new(),
            filenames: Vec::new(),
            mode: ViewMode::Gallery,
            themes: vec![NamedTheme {
                name: "fallback".to_string(),
                path: None,
                theme: theme.clone(),
            }],
            theme,
            theme_index: 0,
            theme_picker: None,
            cell_px,
            graphics: KittyBackend,
            gallery: GalleryState::new(0),
            thumb_cache: ThumbnailCache::new(),
            selection: HashSet::new(),
            pending_delete: None,
            delete_error: None,
            picker: None,
            current_dir: None,
            initial_dir: None,
            current: 0,
            prefetcher: Prefetcher::new(),
            loaded: None,
            loaded_revision: 0,
            error: None,
            needs_render: true,
            image_rect: Rect::default(),
            help_visible: false,
            loaded_for_rect: Rect::default(),
            fullscreen_rx: None,
            fullscreen_target: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            zoom_dirty: false,
            source: None,
            source_for: None,
            source_rx: None,
            source_target: None,
            zoom_rx: None,
            zoom_cache: None,
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
            transmitted_image_ids: HashSet::new(),
            graphics_storage_dirty: false,
        }
    }

    pub fn install_themes(&mut self, themes: Vec<NamedTheme>, selected: usize) {
        if themes.is_empty() {
            return;
        }
        self.theme_index = selected.min(themes.len() - 1);
        self.theme = themes[self.theme_index].theme.clone();
        self.themes = themes;
        self.needs_render = true;
    }

    pub fn open_theme_picker(&mut self) {
        self.theme_picker = Some(ThemePickerState {
            selected: self.theme_index,
            original_index: self.theme_index,
            scroll: 0,
            visible_height: 1,
        });
        self.needs_render = true;
    }

    pub fn theme_picker_select(&mut self, selected: usize) {
        if selected >= self.themes.len() {
            return;
        }
        if let Some(picker) = self.theme_picker.as_mut() {
            picker.selected = selected;
            self.theme_index = selected;
            self.theme = self.themes[selected].theme.clone();
            self.needs_render = true;
        }
    }

    pub fn theme_picker_confirm(&mut self) {
        self.theme_picker = None;
        self.needs_render = true;
    }

    pub fn theme_picker_cancel(&mut self) {
        let Some(picker) = self.theme_picker.take() else {
            return;
        };
        self.theme_index = picker.original_index;
        self.theme = self.themes[self.theme_index].theme.clone();
        self.needs_render = true;
    }

    /// Wipe backend storage and drop transmitted-image tracking.
    pub fn graphics_delete_all(&mut self) -> io::Result<()> {
        self.graphics.delete_all()?;
        self.transmitted_image_ids.clear();
        Ok(())
    }

    pub fn graphics_delete_all_to<W: std::io::Write>(&mut self, out: &mut W) -> io::Result<()> {
        self.graphics.delete_all_to(out)?;
        self.transmitted_image_ids.clear();
        Ok(())
    }

    pub fn refresh_from_scanner(&mut self) {
        let scan_errors = self.shared_list.drain_errors();
        if !scan_errors.is_empty() {
            self.error = Some(scan_errors.join("; "));
            self.needs_render = true;
        }

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
            } else {
                self.update_filter();
            }

            self.known_len = new_len;
            if old_len == 0 {
                self.needs_render = true;
            }
        }

        if !self.scan_complete && self.shared_list.is_complete() {
            self.scan_complete = true;
            self.finalize_scan_order();

            if self.images.len() == 1 && self.mode == ViewMode::Gallery {
                self.mode = ViewMode::Fullscreen;
                self.needs_render = true;
            }
        }
    }

    fn finalize_scan_order(&mut self) {
        if self.images.len() < 2 {
            return;
        }

        let selected_path = self
            .gallery
            .selected_index()
            .and_then(|index| self.images.get(index))
            .cloned();
        let current_path = self.images.get(self.current).cloned();
        let mut media: Vec<(PathBuf, String)> = self
            .images
            .drain(..)
            .zip(self.filenames.drain(..))
            .collect();
        media.sort_by_cached_key(|(path, filename)| (filename.to_lowercase(), path.clone()));
        (self.images, self.filenames) = media.into_iter().unzip();

        self.current = current_path
            .as_ref()
            .and_then(|path| self.images.iter().position(|candidate| candidate == path))
            .unwrap_or(0);
        if self.gallery.search_query.is_empty() {
            self.gallery.filtered_indices = (0..self.images.len()).collect();
            self.gallery.cursor = selected_path
                .as_ref()
                .and_then(|path| self.images.iter().position(|candidate| candidate == path))
                .unwrap_or(0);
            self.gallery.ensure_cursor_visible();
        } else {
            self.update_filter();
        }

        self.shared_list
            .replace_all(self.images.clone(), self.filenames.clone());
        self.known_len = self.images.len();
        self.thumb_cache.clear();
        self.thumb_loading.clear();
        self.thumb_generation += 1;
        self.prefetcher.invalidate();
        self.graphics_storage_dirty = true;
        self.set_loaded(None);
        self.needs_render = true;
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
            self.set_loaded(None);
            self.reset_zoom_state();
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
                    && self.zoom == 1.0
                {
                    match result {
                        Ok(img) => {
                            self.set_loaded(Some(img));
                            self.error = None;
                            self.loaded_for_rect = rect;
                            self.needs_render = true;
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                            self.needs_render = true;
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

    /// Reset zoom/pan and drop the cached full-resolution source. Called when the
    /// current fullscreen image changes.
    fn reset_zoom_state(&mut self) {
        self.zoom = 1.0;
        self.pan_x = 0.5;
        self.pan_y = 0.5;
        self.zoom_dirty = false;
        self.source = None;
        self.source_for = None;
        self.source_rx = None;
        self.source_target = None;
        self.zoom_cache = None;
    }

    /// Return to fit-to-window, keeping the cached source for fast re-zoom.
    pub fn reset_zoom(&mut self) {
        if self.zoom == 1.0 {
            return;
        }
        self.zoom = 1.0;
        self.pan_x = 0.5;
        self.pan_y = 0.5;
        self.zoom_dirty = true;
        self.needs_render = true;
    }

    pub fn zoom_in(&mut self) {
        let new = (self.zoom * ZOOM_STEP).min(MAX_ZOOM);
        if new != self.zoom {
            self.zoom = new;
            self.clamp_pan();
            self.zoom_dirty = true;
            self.needs_render = true;
        }
    }

    pub fn zoom_out(&mut self) {
        let new = (self.zoom / ZOOM_STEP).max(1.0);
        if new != self.zoom {
            self.zoom = new;
            self.clamp_pan();
            self.zoom_dirty = true;
            self.needs_render = true;
        }
    }

    /// Pan the zoomed view. `dx`/`dy` are direction multipliers (usually -1, 0, or 1).
    pub fn pan(&mut self, dx: f64, dy: f64) {
        if self.zoom <= 1.0 {
            return;
        }
        let step = 0.15 / self.zoom;
        self.pan_x += dx * step;
        self.pan_y += dy * step;
        self.clamp_pan();
        self.zoom_dirty = true;
        self.needs_render = true;
    }

    /// Move the image with the pointer by a viewport-pixel displacement.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        if self.zoom <= 1.0 {
            return;
        }
        let Some(src) = self.source.as_ref() else {
            return;
        };
        let (ow, oh) = src.dimensions();
        if ow == 0.0 || oh == 0.0 {
            return;
        }
        let (vw, vh) = self.viewport_pixels();
        let crop = src.crop(self.zoom, vw, vh);
        let previous = (self.pan_x, self.pan_y);
        if crop.max_x > 0.0 {
            self.pan_x -= dx / (crop.max_x * crop.scale);
        }
        if crop.max_y > 0.0 {
            self.pan_y -= dy / (crop.max_y * crop.scale);
        }
        self.clamp_pan();
        if previous != (self.pan_x, self.pan_y) {
            self.zoom_dirty = true;
            self.needs_render = true;
        }
    }

    fn clamp_pan(&mut self) {
        self.pan_x = self.pan_x.clamp(0.0, 1.0);
        self.pan_y = self.pan_y.clamp(0.0, 1.0);
    }

    fn start_source_decode(&mut self, idx: usize) {
        let (tx, rx) = mpsc::channel();
        self.source_rx = Some(rx);
        self.source_target = Some(idx);
        let path = self.images[idx].clone();
        rayon::spawn(move || {
            let source = if is_svg(&path) {
                parse_svg(&path).map(ZoomSource::Svg)
            } else {
                decode_image_with_hint(&path, None).map(ZoomSource::Raster)
            };
            let _ = tx.send(source);
        });
    }

    /// Receive an async raster decode or SVG parse for zooming.
    pub fn poll_source(&mut self) -> bool {
        let result = self.source_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            let idx = self.source_target;
            self.source_rx = None;
            if idx == Some(self.current) {
                match result {
                    Ok(source) => {
                        self.source = Some(Arc::new(source));
                        self.source_for = idx;
                        self.zoom_dirty = true;
                    }
                    Err(error) if self.mode == ViewMode::Fullscreen && self.zoom > 1.0 => {
                        self.set_loaded(None);
                        self.error = Some(error.to_string());
                    }
                    Err(_) => self.source_target = None,
                }
                self.needs_render = true;
            }
            return true;
        }
        false
    }

    /// Poll one background zoom render, discarding obsolete image/geometry results.
    pub fn poll_zoom(&mut self) -> bool {
        let result = self.zoom_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        let Some(result) = result else {
            return false;
        };
        self.zoom_rx = None;
        let same_source = self
            .source
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(source, &result.request.source));
        if same_source && self.mode == ViewMode::Fullscreen && self.zoom > 1.0 {
            self.zoom_cache = result.cache;
            let same_pan = result.request.pan_x == self.pan_x && result.request.pan_y == self.pan_y;
            let same_view = result.request.matches(self);
            if same_view && same_pan {
                match result.image {
                    Ok(image) => {
                        self.set_loaded(Some(image));
                        self.error = None;
                    }
                    Err(error) => {
                        self.set_loaded(None);
                        self.error = Some(error.to_string());
                    }
                }
                self.loaded_for_rect = self.image_rect;
                self.zoom_dirty = false;
            } else if same_view {
                drop(result.image);
                // Re-project a completed SVG canvas to the latest pointer position
                // without another render or publishing an obsolete crop.
                let latest = self.zoom_request();
                let image = self
                    .zoom_cache
                    .as_ref()
                    .and_then(|cache| latest.cached_image(cache).ok().flatten());
                if let Some(image) = image {
                    self.set_loaded(Some(image));
                    self.error = None;
                    self.loaded_for_rect = self.image_rect;
                    self.zoom_dirty = false;
                } else {
                    self.zoom_dirty = true;
                }
            } else {
                self.zoom_dirty = true;
            }
            self.needs_render = true;
        } else if self.mode == ViewMode::Fullscreen && self.zoom > 1.0 {
            self.zoom_dirty = true;
            self.needs_render = true;
        }
        true
    }

    /// Keep the previous graphics visible while source or zoom work is outstanding.
    pub fn zoom_render_pending(&self) -> bool {
        self.zoom > 1.0 && (self.source_rx.is_some() || self.zoom_rx.is_some())
    }

    pub fn enter_gallery(&mut self) {
        self.mode = ViewMode::Gallery;
        self.set_loaded(None);
        self.needs_render = true;
    }

    pub fn toggle_selection_at_cursor(&mut self) {
        let Some(idx) = self.gallery.selected_index() else {
            return;
        };
        let path = self.images[idx].clone();
        if !self.selection.remove(&path) {
            self.selection.insert(path);
        }
    }

    pub fn select_all_filtered(&mut self) {
        for &idx in &self.gallery.filtered_indices {
            self.selection.insert(self.images[idx].clone());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn is_marked(&self, img_idx: usize) -> bool {
        self.images
            .get(img_idx)
            .is_some_and(|p| self.selection.contains(p))
    }

    pub fn begin_delete(&mut self, permanent: bool) {
        let from_fullscreen = matches!(self.mode, ViewMode::Fullscreen);
        #[cfg(feature = "video")]
        let from_fullscreen = from_fullscreen || matches!(self.mode, ViewMode::Video);

        let paths: Vec<PathBuf> = if from_fullscreen {
            if self.current >= self.images.len() {
                return;
            }
            vec![self.images[self.current].clone()]
        } else if !self.selection.is_empty() {
            let mut v: Vec<PathBuf> = self
                .gallery
                .filtered_indices
                .iter()
                .filter_map(|&i| {
                    let p = self.images.get(i)?;
                    if self.selection.contains(p) {
                        Some(p.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if v.is_empty() {
                v = self.selection.iter().cloned().collect();
            }
            v
        } else if let Some(idx) = self.gallery.selected_index() {
            vec![self.images[idx].clone()]
        } else {
            return;
        };

        if paths.is_empty() {
            return;
        }

        self.pending_delete = Some(PendingDelete {
            paths,
            from_fullscreen,
            permanent,
        });
        self.delete_error = None;
    }

    pub fn cancel_delete(&mut self) {
        self.pending_delete.take();
    }

    pub fn confirm_delete(&mut self) {
        let Some(pending) = self.pending_delete.take() else {
            return;
        };

        let mut removed: HashSet<PathBuf> = HashSet::new();
        let mut errors: Vec<String> = Vec::new();
        for p in &pending.paths {
            let result = if pending.permanent {
                std::fs::remove_file(p).map_err(|error| error.to_string())
            } else {
                trash::delete(p).map_err(|error| error.to_string())
            };
            match result {
                Ok(()) => {
                    removed.insert(p.clone());
                }
                Err(error) => {
                    errors.push(format!("{}: {error}", p.display()));
                }
            }
        }

        if removed.is_empty() {
            self.delete_error = Some(errors.join("; "));
            self.needs_render = true;
            return;
        }

        let current_path = self.images.get(self.current).cloned();

        let mut new_images = Vec::with_capacity(self.images.len() - removed.len());
        let mut new_filenames = Vec::with_capacity(self.filenames.len() - removed.len());
        for (i, p) in self.images.iter().enumerate() {
            if !removed.contains(p) {
                new_images.push(p.clone());
                new_filenames.push(self.filenames[i].clone());
            }
        }

        self.images = new_images;
        self.filenames = new_filenames;
        for p in &removed {
            self.selection.remove(p);
        }
        self.known_len = self.images.len();
        self.shared_list
            .replace_all(self.images.clone(), self.filenames.clone());

        self.update_filter();
        if self.gallery.search_query.is_empty() {
            self.gallery.filtered_indices = (0..self.images.len()).collect();
        }

        let cursor = self.gallery.cursor;
        let max_cursor = self.gallery.filtered_indices.len().saturating_sub(1);
        self.gallery.cursor = cursor.min(max_cursor);
        self.gallery.ensure_cursor_visible();

        self.thumb_cache.clear();
        self.thumb_loading.clear();
        self.thumb_generation += 1;
        self.prefetcher.invalidate();
        self.graphics_storage_dirty = true;
        self.fullscreen_rx = None;
        self.fullscreen_target = None;
        self.set_loaded(None);

        self.delete_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        };

        if pending.from_fullscreen {
            if self.images.is_empty() {
                self.mode = ViewMode::Gallery;
            } else {
                let old_current = current_path
                    .as_ref()
                    .and_then(|p| self.images.iter().position(|q| q == p));
                let target = old_current.unwrap_or_else(|| self.current.min(self.images.len() - 1));
                self.jump_to(target);
            }
        } else if self.images.is_empty() {
            self.current = 0;
        } else if self.current >= self.images.len() {
            self.current = self.images.len() - 1;
        }

        self.needs_render = true;
    }

    pub fn open_picker(&mut self) {
        let start = self
            .current_dir
            .clone()
            .or_else(|| {
                self.initial_dir
                    .clone()
                    .map(|p| picker::canonicalize_or_self(&p))
            })
            .unwrap_or_else(|| picker::initial_picker_dir(&self.images));
        self.current_dir = Some(start.clone());
        self.picker = Some(PickerState::new(start));
        self.mode = ViewMode::Picker;
        self.needs_render = true;
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
        self.mode = ViewMode::Gallery;
        self.needs_render = true;
    }

    /// Replace image list with the contents of `dir` and reset all view state.
    /// Spawns a fresh scanner thread; the previous scanner (if any) is orphaned
    /// against an unused SharedImageList clone and will be dropped as it completes.
    pub fn switch_to_dir(&mut self, dir: PathBuf) {
        let canonical = picker::canonicalize_or_self(&dir);
        self.current_dir = Some(canonical.clone());

        let new_list = SharedImageList::new();
        scanner::spawn(vec![canonical.clone()], new_list.clone());
        self.shared_list = new_list;

        self.images.clear();
        self.filenames.clear();
        self.known_len = 0;
        self.scan_complete = false;
        self.selection.clear();
        self.pending_delete = None;
        self.delete_error = None;

        self.gallery.search_active = false;
        self.gallery.search_query.clear();
        self.gallery.filtered_indices.clear();
        self.gallery.reset_cursor();
        self.filter_rx = None;

        self.current = 0;
        self.thumb_cache.clear();
        self.thumb_loading.clear();
        self.thumb_generation += 1;
        self.prefetcher.invalidate();
        self.graphics_storage_dirty = true;
        self.fullscreen_rx = None;
        self.fullscreen_target = None;
        self.set_loaded(None);
        self.error = None;

        if let Some(ref mut p) = self.picker {
            p.current_dir = canonical;
            p.load_dir();
            p.adjust_scroll(p.filtered_indices.len().max(1));
        }

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
        self.reset_zoom_state();

        #[cfg(feature = "video")]
        if crate::video::is_video(&self.images[self.current]) {
            self.mode = ViewMode::Video;
            self.set_loaded(None);
            return;
        }

        self.mode = ViewMode::Fullscreen;
        if let Some(img) = self
            .prefetcher
            .take_resized(self.current, self.image_rect, self.cell_px)
        {
            self.set_loaded(Some(img));
            self.loaded_for_rect = self.image_rect;
        } else {
            self.set_loaded(None);
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
        self.set_loaded(None);
        self.thumb_cache.clear();
        self.thumb_loading.clear();
        self.thumb_generation += 1;
        self.prefetcher.invalidate();
        self.graphics_storage_dirty = true;
        self.needs_render = true;
    }

    pub fn load_if_needed(&mut self) {
        // Zoom from the original raster pixels or render the retained SVG vectors.
        if self.zoom > 1.0 {
            if self.source_for == Some(self.current) {
                if self.zoom_dirty
                    || (self.loaded.is_none() && self.error.is_none())
                    || self.loaded_for_rect != self.image_rect
                {
                    if self.zoom_rx.is_none() {
                        let request = self.zoom_request();
                        let mut cache = self.zoom_cache.take();
                        let (tx, rx) = mpsc::channel();
                        self.zoom_rx = Some(rx);
                        rayon::spawn(move || {
                            let image = request.render(&mut cache);
                            let _ = tx.send(ZoomResult {
                                request,
                                cache,
                                image,
                            });
                        });
                        self.zoom_dirty = false;
                    }
                }
                return;
            }
            // Source not decoded yet: request it and keep showing the fit image
            // until it arrives (poll_source will trigger the rebuild).
            if self.source_target != Some(self.current) {
                self.start_source_decode(self.current);
            }
            if self.source_rx.is_none() {
                // Keep an active source failure visible instead of replacing it
                // with another fit decode.
                self.zoom_dirty = false;
                return;
            }
            self.zoom_dirty = false;
            if self.loaded.is_some() {
                return;
            }
        } else if self.source_rx.is_none() {
            self.source_target = None;
        }

        if self.loaded.is_some() && self.loaded_for_rect == self.image_rect && !self.zoom_dirty {
            return;
        }
        self.zoom_dirty = false;

        if let Some(img) = self
            .prefetcher
            .take_resized(self.current, self.image_rect, self.cell_px)
        {
            self.set_loaded(Some(img));
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

    fn viewport_pixels(&self) -> (u32, u32) {
        (
            (self.image_rect.width as u32 * self.cell_px.0).max(1),
            (self.image_rect.height as u32 * self.cell_px.1).max(1),
        )
    }

    fn set_loaded(&mut self, image: Option<RgbaImage>) {
        self.loaded = image;
        self.loaded_revision = self.loaded_revision.wrapping_add(1);
    }

    fn zoom_request(&self) -> ZoomRequest {
        ZoomRequest {
            source: Arc::clone(self.source.as_ref().expect("loaded zoom source")),
            index: self.current,
            rect: self.image_rect,
            cell_px: self.cell_px,
            zoom: self.zoom,
            pan_x: self.pan_x,
            pan_y: self.pan_y,
        }
    }

    #[cfg(test)]
    fn build_zoom_view(&self) -> io::Result<RgbaImage> {
        self.zoom_request().render(&mut None)
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

/// Resize `img` to exactly `dst_w` x `dst_h` (used for zoomed crops, which may upscale).
pub(crate) fn resize_to_exact(img: &DynamicImage, dst_w: u32, dst_h: u32) -> RgbaImage {
    let (ow, oh) = (img.width(), img.height());
    if dst_w == 0 || dst_h == 0 || ow == 0 || oh == 0 {
        return img.to_rgba8();
    }
    if dst_w == ow && dst_h == oh {
        return img.to_rgba8();
    }
    let src_rgba = img.to_rgba8();
    let Ok(src_image) =
        fir::images::Image::from_vec_u8(ow, oh, src_rgba.into_raw(), fir::PixelType::U8x4)
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

fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

pub(crate) fn decode_image_with_hint(
    path: &Path,
    target: Option<(u32, u32)>,
) -> io::Result<DynamicImage> {
    if is_svg(path) {
        return decode_svg(path, target);
    }

    // libjpeg-turbo is the fast path, but it rejects some JPEG variants the
    // pure-Rust decoder accepts (and vice versa), so a turbo failure falls
    // through to `image` rather than aborting. Remember turbo's error so that if
    // the fallback also fails we can report *both* reasons instead of only the
    // fallback's — otherwise a genuine turbo diagnosis is lost.
    #[cfg(feature = "turbo")]
    let turbo_error = if is_jpeg(path) {
        match decode_jpeg_turbo(path, target) {
            Ok(img) => return Ok(img),
            Err(error) => Some(error),
        }
    } else {
        None
    };

    let fallback = ImageReader::open(path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?
        .with_guessed_format()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        .decode();

    match fallback {
        Ok(img) => Ok(img),
        Err(fallback_error) => {
            #[cfg(feature = "turbo")]
            if let Some(turbo_error) = turbo_error {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "failed to decode image (libjpeg-turbo: {turbo_error}; image: {fallback_error})"
                    ),
                ));
            }
            Err(io::Error::new(io::ErrorKind::InvalidData, fallback_error))
        }
    }
}

fn parse_svg(path: &Path) -> io::Result<resvg::usvg::Tree> {
    use resvg::usvg;

    static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });
    let options = usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        fontdb: Arc::clone(&FONTS),
        ..usvg::Options::default()
    };
    let data = std::fs::read(path)?;
    usvg::Tree::from_data(&data, &options)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn decode_svg(path: &Path, target: Option<(u32, u32)>) -> io::Result<DynamicImage> {
    let tree = parse_svg(path)?;
    let size = tree.size();
    // Match raster fit-to-window behavior: preserve aspect ratio, never upscale.
    let scale = match target {
        Some((w, h)) if w > 0 && h > 0 => (w as f32 / size.width())
            .min(h as f32 / size.height())
            .min(1.0),
        _ => 1.0,
    };
    let width = ((size.width() * scale) as u32).max(1);
    let height = ((size.height() * scale) as u32).max(1);
    render_svg(
        &tree,
        width,
        height,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        None,
    )
    .map(DynamicImage::ImageRgba8)
}

struct SvgViewport {
    crop: resvg::tiny_skia::IntRect,
    transform: resvg::tiny_skia::Transform,
    full_width: u32,
    full_height: u32,
    origin: (f64, f64),
}

impl SvgViewport {
    fn new(
        tree: &resvg::usvg::Tree,
        width: u32,
        height: u32,
        scale: f64,
        origin: (f64, f64),
    ) -> io::Result<Self> {
        // Preserve fractional phase before narrowing large translations to f32.
        let snap_roundoff = |value: f64| {
            let rounded = value.round();
            if (value - rounded).abs() <= value.abs().max(1.0) * f64::EPSILON * 16.0 {
                rounded
            } else {
                value
            }
        };
        let x = snap_roundoff(origin.0);
        let y = snap_roundoff(origin.1);
        let left = x.ceil().max(0.0) as u32;
        let top = y.ceil().max(0.0) as u32;
        let crop = resvg::tiny_skia::IntRect::from_xywh(left as i32, top as i32, width, height)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid SVG crop dimensions")
            })?;
        let transform = resvg::tiny_skia::Transform::from_row(
            scale as f32,
            0.0,
            0.0,
            scale as f32,
            (f64::from(left) - x) as f32,
            (f64::from(top) - y) as f32,
        );
        let full_width = ((tree.size().width() * transform.sx + transform.tx).ceil() as u32)
            .max(crop.right() as u32);
        let full_height = ((tree.size().height() * transform.sy + transform.ty).ceil() as u32)
            .max(crop.bottom() as u32);
        Ok(Self {
            crop,
            transform,
            full_width,
            full_height,
            origin: (x, y),
        })
    }

    fn cached_image(&self, cache: &SvgZoomCache) -> Option<RgbaImage> {
        if cache.transform != self.transform
            || cache.image.width() != self.full_width
            || cache.image.height() != self.full_height
        {
            return None;
        }
        Some(
            image::imageops::crop_imm(
                &cache.image,
                self.crop.x() as u32,
                self.crop.y() as u32,
                self.crop.width(),
                self.crop.height(),
            )
            .to_image(),
        )
    }
}

/// Cache exact-resolution straight RGBA so whole-pixel panning only copies its crop.
fn render_svg_cached(
    tree: &resvg::usvg::Tree,
    width: u32,
    height: u32,
    scale: f64,
    origin: (f64, f64),
    cache: &mut Option<SvgZoomCache>,
) -> io::Result<RgbaImage> {
    let view = SvgViewport::new(tree, width, height, scale, origin)?;
    if let Err(error) = reserve_svg_buffers(view.full_width, view.full_height, Some(view.crop)) {
        *cache = None;
        if tree.filters().is_empty() {
            // Large unfiltered drawings still render only their visible viewport.
            let viewport_transform = resvg::tiny_skia::Transform::from_row(
                scale as f32,
                0.0,
                0.0,
                scale as f32,
                -view.origin.0 as f32,
                -view.origin.1 as f32,
            );
            return render_svg(tree, width, height, viewport_transform, None);
        }
        return Err(error);
    }
    if let Some(image) = cache.as_ref().and_then(|cache| view.cached_image(cache)) {
        return Ok(image);
    }
    // Release the old canvas before allocating a new zoom/phase.
    *cache = None;
    let image = rgba_from_pixmap(render_svg_pixmap(
        tree,
        view.full_width,
        view.full_height,
        view.transform,
    )?)?;
    *cache = Some(SvgZoomCache {
        transform: view.transform,
        image,
    });
    Ok(view.cached_image(cache.as_ref().unwrap()).unwrap())
}

fn render_svg(
    tree: &resvg::usvg::Tree,
    width: u32,
    height: u32,
    transform: resvg::tiny_skia::Transform,
    crop: Option<resvg::tiny_skia::IntRect>,
) -> io::Result<RgbaImage> {
    reserve_svg_buffers(width, height, crop)?;
    let pixmap = render_svg_pixmap(tree, width, height, transform)?;
    let pixmap = if let Some(crop) = crop {
        pixmap.clone_rect(crop).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid SVG crop dimensions")
        })?
    } else {
        pixmap
    };
    rgba_from_pixmap(pixmap)
}

fn reserve_svg_buffers(
    width: u32,
    height: u32,
    crop: Option<resvg::tiny_skia::IntRect>,
) -> io::Result<()> {
    let mut limits = image::Limits::default();
    limits
        .reserve_buffer(width, height, image::ColorType::Rgba8)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if let Some(crop) = crop {
        limits
            .reserve_buffer(crop.width(), crop.height(), image::ColorType::Rgba8)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    Ok(())
}

fn render_svg_pixmap(
    tree: &resvg::usvg::Tree,
    width: u32,
    height: u32,
    transform: resvg::tiny_skia::Transform,
) -> io::Result<resvg::tiny_skia::Pixmap> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid SVG image dimensions")
    })?;
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

fn rgba_from_pixmap(pixmap: resvg::tiny_skia::Pixmap) -> io::Result<RgbaImage> {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut pixels = pixmap.take();
    // tiny-skia produces premultiplied RGBA; image and Kitty need straight alpha.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha) as u8;
            }
        }
    }
    RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SVG image dimensions"))
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
    // SVGs may reference other files or system fonts not covered by this key.
    if is_svg(path) {
        return None;
    }

    use std::collections::hash_map::DefaultHasher;
    use std::fs;
    use std::hash::{Hash, Hasher};

    let metadata = fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?;
    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    let mut hasher = DefaultHasher::new();
    "rview-thumbnail-v2".hash(&mut hasher);
    canonical_path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    mtime.as_secs().hash(&mut hasher);
    mtime.subsec_nanos().hash(&mut hasher);
    target_w.hash(&mut hasher);
    target_h.hash(&mut hasher);
    let filename = format!("{:016x}.png", hasher.finish());

    let cache_dir = ProjectDirs::from("", "", "rview")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("rview-cache"));

    Some(cache_dir.join(filename))
}

fn try_load_from_disk_cache(path: &Path, target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let cache_path = get_cache_file_path(path, target_w, target_h)?;
    if cache_path.exists() {
        match image::open(&cache_path) {
            Ok(image) => Some(image.to_rgba8()),
            Err(_) => {
                let _ = std::fs::remove_file(cache_path);
                None
            }
        }
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
    static CACHE_PRUNED: OnceLock<()> = OnceLock::new();
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let cache_path = get_cache_file_path(path, target_w, target_h)?;
    let parent = cache_path.parent()?;
    fs::create_dir_all(parent).ok()?;
    CACHE_PRUNED.get_or_init(|| prune_disk_cache(parent, 512 * 1024 * 1024));
    if cache_path.exists() {
        return Some(());
    }

    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let cache_name = cache_path.file_name()?.to_string_lossy();
    let temp_path = parent.join(format!(
        ".{cache_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    if img
        .save_with_format(&temp_path, image::ImageFormat::Png)
        .is_err()
    {
        let _ = fs::remove_file(temp_path);
        return None;
    }
    if fs::rename(&temp_path, &cache_path).is_err() {
        let _ = fs::remove_file(temp_path);
        if !cache_path.exists() {
            return None;
        }
    }
    if sequence % 64 == 0 {
        prune_disk_cache(parent, 512 * 1024 * 1024);
    }
    Some(())
}

fn prune_disk_cache(cache_dir: &Path, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
        .flatten()
        .filter_map(|entry| {
            if entry.path().extension().and_then(|ext| ext.to_str()) != Some("png") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    entry.path(),
                    metadata.len(),
                    metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                )
            })
        })
        .collect();
    let mut total_bytes: u64 = files.iter().map(|(_, bytes, _)| bytes).sum();
    if total_bytes <= max_bytes {
        return;
    }

    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, bytes, _) in files {
        if total_bytes <= max_bytes {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total_bytes = total_bytes.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_image_with_hint, get_cache_file_path, resize_decoded_to_dims, resize_to_exact,
    };
    use image::{DynamicImage, GenericImageView};
    use std::fs;

    fn load_zoom(app: &mut super::App) {
        use std::time::{Duration, Instant};
        app.mode = super::ViewMode::Fullscreen;
        let deadline = Instant::now() + Duration::from_secs(10);
        app.load_if_needed();
        while app.zoom_render_pending() {
            assert!(Instant::now() < deadline, "zoom render timed out");
            std::thread::sleep(Duration::from_millis(1));
            app.poll_zoom();
            app.load_if_needed();
        }
    }

    #[test]
    fn obsolete_zoom_results_do_not_overwrite_new_pan_or_image() {
        use super::{App, ZoomResult, ZoomSource};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;
        use std::sync::{Arc, mpsc};

        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 100, 100);
        app.source = Some(Arc::new(ZoomSource::Raster(DynamicImage::ImageRgba8(
            image::RgbaImage::from_fn(200, 100, |x, _| image::Rgba([x as u8, 0, 0, 255])),
        ))));
        app.source_for = Some(0);
        app.zoom = 2.0;
        load_zoom(&mut app);

        app.pan_pixels(-10.0, 0.0);
        let request = app.zoom_request();
        let image = request.render(&mut None);
        let (tx, rx) = mpsc::channel();
        app.zoom_rx = Some(rx);
        app.pan_pixels(-10.0, 0.0);
        tx.send(ZoomResult {
            request,
            cache: None,
            image,
        })
        .unwrap();
        app.poll_zoom();
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(0, 0).0,
            [50, 0, 0, 255]
        );
        load_zoom(&mut app);
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(0, 0).0,
            [70, 0, 0, 255]
        );

        let request = app.zoom_request();
        let image = request.render(&mut None);
        let (tx, rx) = mpsc::channel();
        app.zoom_rx = Some(rx);
        app.current = 1;
        app.source = Some(Arc::new(ZoomSource::Raster(DynamicImage::ImageRgba8(
            image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 255, 0, 255])),
        ))));
        app.source_for = Some(1);
        app.set_loaded(None);
        tx.send(ZoomResult {
            request,
            cache: None,
            image,
        })
        .unwrap();
        app.poll_zoom();
        assert!(app.loaded.is_none());
        load_zoom(&mut app);
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(0, 0).0,
            [0, 255, 0, 255]
        );
    }

    #[test]
    fn completed_svg_canvas_tracks_latest_pan_without_publishing_old_crops() {
        use super::{App, ZoomResult, ZoomSource};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;
        use std::sync::{Arc, mpsc};

        let tree = resvg::usvg::Tree::from_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <rect x="50" width="10" height="100" fill="red"/></svg>"#,
            &resvg::usvg::Options::default(),
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 100, 100);
        app.source = Some(Arc::new(ZoomSource::Svg(tree)));
        app.source_for = Some(0);
        app.zoom = 2.0;
        load_zoom(&mut app);

        app.pan_pixels(-10.0, 0.0);
        let request = app.zoom_request();
        let mut cache = app.zoom_cache.take();
        let image = request.render(&mut cache);
        let (tx, rx) = mpsc::channel();
        app.zoom_rx = Some(rx);
        app.pan_pixels(-10.0, 0.0);
        tx.send(ZoomResult {
            request,
            cache,
            image,
        })
        .unwrap();
        app.poll_zoom();
        let latest = app.loaded.as_ref().unwrap();
        assert_eq!(latest.get_pixel(30, 50).0, [255, 0, 0, 255]);
        assert_eq!(latest.get_pixel(50, 50).0, [0, 0, 0, 0]);

        let previous = latest.clone();
        let request = app.zoom_request();
        let mut cache = app.zoom_cache.take();
        let image = request.render(&mut cache);
        let (tx, rx) = mpsc::channel();
        app.zoom_rx = Some(rx);
        app.pan_pixels(-0.5, 0.0);
        tx.send(ZoomResult {
            request,
            cache,
            image,
        })
        .unwrap();
        app.poll_zoom();
        assert_eq!(app.loaded.as_ref(), Some(&previous));
        load_zoom(&mut app);
        let alpha = app.loaded.as_ref().unwrap().get_pixel(29, 50).0[3];
        assert!(alpha > 0 && alpha < 255);
    }

    #[test]
    fn late_source_errors_preserve_fit_but_report_active_zoom_failure() {
        use super::{App, ViewMode};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use std::{io, sync::mpsc};

        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.mode = ViewMode::Fullscreen;
        app.zoom = 2.0;
        let (tx, rx) = mpsc::channel();
        app.source_target = Some(app.current);
        app.source_rx = Some(rx);
        app.reset_zoom();
        let fit = image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 255, 0, 255]));
        app.set_loaded(Some(fit.clone()));
        tx.send(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zoom decode failed",
        )))
        .unwrap();
        app.poll_source();
        assert_eq!(app.loaded.as_ref(), Some(&fit));
        assert!(app.error.is_none());

        let (tx, rx) = mpsc::channel();
        app.source_rx = Some(rx);
        app.source_target = Some(app.current);
        app.zoom = 2.0;
        tx.send(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zoom decode failed",
        )))
        .unwrap();
        app.poll_source();
        app.load_if_needed();
        assert!(app.loaded.is_none());
        assert_eq!(app.error.as_deref(), Some("zoom decode failed"));
    }

    #[test]
    fn cached_svg_pan_preserves_straight_alpha_and_fractional_edges() {
        use super::{App, ZoomSource};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;
        use std::sync::Arc;

        let tree = resvg::usvg::Tree::from_str(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1">
                <g opacity="0.5"><rect width="1" height="1" fill="#804020"/>
                <rect x="1" width="1" height="1" fill="blue"/></g></svg>"##,
            &resvg::usvg::Options::default(),
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 8, 8);
        app.source = Some(Arc::new(ZoomSource::Svg(tree)));
        app.source_for = Some(0);
        app.zoom = 16.0;
        load_zoom(&mut app);
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(3, 4).0,
            [128, 64, 32, 128]
        );
        app.pan_pixels(2.0, 0.0);
        load_zoom(&mut app);
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(5, 4).0,
            [128, 64, 32, 128]
        );
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(6, 4).0,
            [0, 0, 255, 128]
        );
        app.pan_pixels(0.5, 0.0);
        load_zoom(&mut app);
        let edge = app.loaded.as_ref().unwrap().get_pixel(6, 4).0;
        assert!(edge[0] > 0 && edge[0] < 128);
        assert!(edge[2] > 32 && edge[2] < 255);
    }

    #[test]
    fn svg_renders_viewbox_at_target_size_with_straight_alpha() {
        let path =
            std::env::temp_dir().join(format!("rview-synthetic-alpha-{}.SVG", std::process::id()));
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">
                <rect width="50" height="50" fill="#ff0000" fill-opacity="0.5"/>
            </svg>"##,
        )
        .unwrap();

        let thumbnail = decode_image_with_hint(&path, Some((20, 20)))
            .unwrap()
            .into_rgba8();
        let source = decode_image_with_hint(&path, None).unwrap();
        let larger = decode_image_with_hint(&path, Some((200, 200))).unwrap();
        let zero_target = decode_image_with_hint(&path, Some((0, 20))).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(thumbnail.dimensions(), (20, 10));
        assert_eq!(thumbnail.get_pixel(5, 5).0, [255, 0, 0, 128]);
        assert_eq!(thumbnail.get_pixel(15, 5).0, [0, 0, 0, 0]);
        assert_eq!(source.dimensions(), (100, 50));
        assert_eq!(larger.dimensions(), (100, 50));
        assert_eq!(zero_target.dimensions(), (100, 50));
    }

    #[test]
    fn svg_resolves_images_relative_to_its_directory() {
        let root = std::env::temp_dir().join(format!(
            "rview-synthetic-svg-resources-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 255]))
            .save(root.join("tile.png"))
            .unwrap();
        let path = root.join("image.svg");
        fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg"
                xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
                <image width="10" height="10" xlink:href="tile.png"/>
            </svg>"#,
        )
        .unwrap();
        let image = decode_image_with_hint(&path, None).unwrap().into_rgba8();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(image.get_pixel(5, 5).0, [0, 255, 0, 255]);
    }

    #[test]
    fn svg_thumbnail_refreshes_when_a_referenced_image_changes() {
        use super::App;
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let root = std::env::temp_dir().join(format!(
            "rview-synthetic-svg-thumbnail-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let resource = root.join("tile.png");
        let source = root.join("image.SVG");
        fs::write(
            &source,
            r#"<svg xmlns="http://www.w3.org/2000/svg"
                xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
                <image width="10" height="10" xlink:href="tile.png"/>
            </svg>"#,
        )
        .unwrap();
        let mut pixels = Vec::new();
        for color in [[0, 255, 0, 255], [0, 0, 255, 255]] {
            image::RgbaImage::from_pixel(10, 10, image::Rgba(color))
                .save(&resource)
                .unwrap();
            // A new viewer must not reuse a thumbnail of the old reference.
            let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
            app.spawn_thumb_decode(0, source.clone(), Rect::new(0, 0, 10, 10));
            let (_, _, image) = app
                .thumb_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            pixels.push(image.get_pixel(5, 5).0);
        }
        if let Some(cache) = get_cache_file_path(&source, 10, 10) {
            let _ = fs::remove_file(cache);
        }
        fs::remove_dir_all(root).unwrap();
        assert_eq!(pixels, [[0, 255, 0, 255], [0, 0, 255, 255]]);
    }

    #[test]
    fn oversized_svg_rejects_intrinsic_decode_but_allows_bounded_rendering() {
        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-oversized-{}.svg",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg"
                width="100000000" height="100000000">
                <rect width="100000000" height="100000000" fill="red"/>
            </svg>"#,
        )
        .unwrap();
        let intrinsic = decode_image_with_hint(&path, None).unwrap_err();
        let oversized_target =
            decode_image_with_hint(&path, Some((100000000, 100000000))).unwrap_err();
        let bounded = decode_image_with_hint(&path, Some((20, 20)))
            .unwrap()
            .into_rgba8();
        fs::remove_file(path).unwrap();
        assert_eq!(intrinsic.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(oversized_target.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(bounded.dimensions(), (20, 20));
        assert_eq!(bounded.get_pixel(10, 10).0, [255, 0, 0, 255]);
    }

    #[test]
    fn svg_zoom_preserves_filter_inputs_outside_the_viewport() {
        use super::{App, ZoomSource};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let tree = resvg::usvg::Tree::from_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <defs><filter id="f" filterUnits="userSpaceOnUse"
                    primitiveUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">
                    <feOffset in="SourceGraphic" dx="50" dy="0"/>
                </filter></defs>
                <rect x="0" y="45" width="10" height="10" fill="red" filter="url(#f)"/>
            </svg>"#,
            &resvg::usvg::Options::default(),
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 100, 100);
        app.source = Some(std::sync::Arc::new(ZoomSource::Svg(tree)));
        app.zoom = 16.0;
        let view = app.build_zoom_view().unwrap();
        assert_eq!(view.get_pixel(75, 50).0, [255, 0, 0, 255]);
        assert_eq!(view.get_pixel(25, 50).0, [0, 0, 0, 0]);
        app.pan_pixels(0.5, 0.0);
        let shifted = app.build_zoom_view().unwrap();
        assert!(shifted.get_pixel(50, 50).0[3] > 0);
        assert!(shifted.get_pixel(50, 50).0[3] < 255);
        assert_eq!(shifted.get_pixel(51, 50).0, [255, 0, 0, 255]);
    }

    #[test]
    fn svg_zoom_preserves_detail_smaller_than_an_intrinsic_pixel() {
        use super::App;
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;
        use std::fmt::Write;
        use std::time::{Duration, Instant};

        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-svg-zoom-detail-{}.svg",
            std::process::id()
        ));
        let mut svg = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
                <rect width="64" height="64" fill="white"/>"#,
        );
        for stripe in 0..256 {
            write!(
                svg,
                r#"<rect x="{}" width="0.125" height="64" fill="black"/>"#,
                f64::from(stripe) * 0.25
            )
            .unwrap();
        }
        svg.push_str("</svg>");
        fs::write(&path, svg).unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.images.push(path.clone());
        app.image_rect = Rect::new(0, 0, 64, 64);
        app.zoom = 8.0;
        app.start_source_decode(0);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.poll_source() {
            assert!(Instant::now() < deadline, "SVG source decode timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        load_zoom(&mut app);
        let view = app.loaded.as_ref().unwrap();
        fs::remove_file(path).unwrap();

        for x in 0..64 {
            let value = if x % 2 == 0 { 0 } else { 255 };
            assert_eq!(view.get_pixel(x, 32).0, [value, value, value, 255]);
        }
    }

    #[test]
    fn svg_zoom_pans_fractional_source_extents_with_straight_alpha() {
        use super::{App, ZoomSource, parse_svg};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-svg-fractional-pan-{}.svg",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1">
                <g opacity="0.5">
                    <rect width="1" height="1" fill="red"/>
                    <rect x="1" width="1" height="1" fill="blue"/>
                </g>
            </svg>"#,
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.source = Some(std::sync::Arc::new(ZoomSource::Svg(
            parse_svg(&path).unwrap(),
        )));
        fs::remove_file(path).unwrap();
        app.image_rect = Rect::new(0, 0, 8, 8);
        app.zoom = 16.0;
        let view = app.build_zoom_view().unwrap();
        assert_eq!(view.get_pixel(3, 4).0, [255, 0, 0, 128]);
        assert_eq!(view.get_pixel(4, 4).0, [0, 0, 255, 128]);
        app.pan_pixels(2.0, 0.0);
        let dragged = app.build_zoom_view().unwrap();
        assert_eq!(dragged.get_pixel(5, 4).0, [255, 0, 0, 128]);
        assert_eq!(dragged.get_pixel(6, 4).0, [0, 0, 255, 128]);
        app.pan_pixels(-100.0, 0.0);
        assert_eq!(
            app.build_zoom_view().unwrap().get_pixel(0, 4).0,
            [0, 0, 255, 128]
        );
        app.pan_pixels(100.0, 0.0);
        assert_eq!(
            app.build_zoom_view().unwrap().get_pixel(7, 4).0,
            [255, 0, 0, 128]
        );
    }

    #[test]
    fn svg_filter_zoom_reports_canvas_limit_and_recovers() {
        use super::{App, ZoomSource};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let tree = resvg::usvg::Tree::from_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg"
                width="100000000" height="100000000">
                <defs><filter id="f" filterUnits="userSpaceOnUse"
                    x="0" y="0" width="100000000" height="100000000">
                    <feFlood flood-color="red"/>
                </filter></defs>
                <rect width="100000000" height="100000000" filter="url(#f)"/>
            </svg>"#,
            &resvg::usvg::Options::default(),
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 1000, 1000);
        app.source = Some(std::sync::Arc::new(ZoomSource::Svg(tree)));
        app.source_for = Some(0);
        app.zoom = 16.0;
        app.zoom_dirty = true;
        load_zoom(&mut app);
        assert!(app.loaded.is_none());
        assert!(app.error.is_some());

        app.zoom = 1.25;
        app.zoom_dirty = true;
        load_zoom(&mut app);
        let view = app.loaded.as_ref().unwrap();
        assert_eq!(view.dimensions(), (1000, 1000));
        assert_eq!(view.get_pixel(500, 500).0, [255, 0, 0, 255]);
        assert!(app.error.is_none());
    }

    #[test]
    fn svg_zoom_allocates_only_the_visible_viewport() {
        use super::{App, ZoomSource, parse_svg};
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-svg-bounded-zoom-{}.svg",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg"
                width="100000000" height="100000000">
                <rect width="100000000" height="100000000" fill="red"/>
            </svg>"#,
        )
        .unwrap();
        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.source = Some(std::sync::Arc::new(ZoomSource::Svg(
            parse_svg(&path).unwrap(),
        )));
        app.source_for = Some(0);
        fs::remove_file(path).unwrap();
        app.image_rect = Rect::new(0, 0, 20, 20);
        app.zoom = 8.0;
        load_zoom(&mut app);
        let view = app.loaded.as_ref().unwrap();
        assert_eq!(view.dimensions(), (20, 20));
        assert_eq!(view.get_pixel(10, 10).0, [255, 0, 0, 255]);

        // A large output, unlike a large intrinsic size, must still respect the budget.
        app.cell_px = (1000000, 1000000);
        assert_eq!(
            app.build_zoom_view().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        app.zoom_dirty = true;
        load_zoom(&mut app);
        assert!(app.loaded.is_none());
        assert!(app.error.is_some());
        app.cell_px = (1, 1);
        app.zoom_dirty = true;
        load_zoom(&mut app);
        assert_eq!(
            app.loaded.as_ref().unwrap().get_pixel(10, 10).0,
            [255, 0, 0, 255]
        );
        assert!(app.error.is_none());
    }

    #[test]
    fn malformed_svg_returns_invalid_data() {
        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-malformed-{}.svg",
            std::process::id()
        ));
        fs::write(&path, "<svg").unwrap();
        let error = decode_image_with_hint(&path, None).unwrap_err();
        fs::remove_file(path).unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(feature = "turbo")]
    #[test]
    fn corrupt_jpeg_reports_both_decoder_errors() {
        // A file with the JPEG SOI magic but no valid frame: both libjpeg-turbo
        // and the pure-Rust fallback reject it. The error must mention both so a
        // turbo-only diagnosis is not silently lost.
        let path = std::env::temp_dir().join(format!(
            "rview-synthetic-corrupt-{}.jpg",
            std::process::id()
        ));
        fs::write(&path, [0xFF, 0xD8, 0xFF, 0xD9]).unwrap();
        let err = decode_image_with_hint(&path, None).unwrap_err();
        let message = err.to_string();
        fs::remove_file(&path).unwrap();
        assert!(message.contains("libjpeg-turbo:"), "message: {message}");
        assert!(message.contains("image:"), "message: {message}");
    }

    #[test]
    fn resize_preserves_aspect_ratio_without_upscaling() {
        let image = DynamicImage::new_rgba8(100, 50);
        assert_eq!(
            resize_decoded_to_dims(&image, 20, 20).dimensions(),
            (20, 10)
        );
        assert_eq!(
            resize_decoded_to_dims(&image, 200, 200).dimensions(),
            (100, 50)
        );
    }

    #[test]
    fn resize_to_exact_upscales_to_requested_dimensions() {
        let image = DynamicImage::new_rgba8(10, 10);
        // Zoomed crops may upscale, unlike the fit-to-window path.
        assert_eq!(resize_to_exact(&image, 40, 30).dimensions(), (40, 30));
        assert_eq!(resize_to_exact(&image, 10, 10).dimensions(), (10, 10));
    }

    #[test]
    fn zoom_view_crops_and_fills_the_viewport() {
        use super::App;
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.image_rect = Rect::new(0, 0, 100, 100);
        // 400x400 source, viewport 100x100 -> fit scale 0.25 (base image is 100x100).
        app.source = Some(std::sync::Arc::new(super::ZoomSource::Raster(
            DynamicImage::new_rgba8(400, 400),
        )));
        app.source_for = Some(0);
        app.current = 0;

        // At 2x zoom the effective scale is 0.5, so the crop is 200x200 source px
        // upscaled to exactly the 100x100 viewport.
        app.zoom = 2.0;
        let view = app.build_zoom_view().expect("zoom view");
        assert_eq!(view.dimensions(), (100, 100));
    }

    #[test]
    fn mouse_pan_tracks_pixels_and_clamps_at_image_edges() {
        use super::App;
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 100, 100);
        app.source = Some(std::sync::Arc::new(super::ZoomSource::Raster(
            DynamicImage::ImageRgba8(image::RgbaImage::from_fn(200, 100, |x, y| {
                image::Rgba([x as u8, y as u8, 0, 255])
            })),
        )));
        app.zoom = 2.0;

        let before = app.build_zoom_view().unwrap();
        assert_eq!(before.get_pixel(0, 0).0, [50, 0, 0, 255]);
        app.pan_pixels(10.0, 20.0);
        let dragged = app.build_zoom_view().unwrap();
        assert_eq!(dragged.get_pixel(0, 0).0, [40, 0, 0, 255]);
        assert_eq!(dragged.get_pixel(99, 99).0, [139, 99, 0, 255]);

        app.pan_pixels(1000.0, 0.0);
        assert_eq!(
            app.build_zoom_view().unwrap().get_pixel(0, 0).0,
            [0, 0, 0, 255]
        );
        app.pan_pixels(-1000.0, 0.0);
        assert_eq!(
            app.build_zoom_view().unwrap().get_pixel(0, 0).0,
            [100, 0, 0, 255]
        );
        app.reset_zoom();
        let fit = app.build_zoom_view().unwrap();
        app.pan_pixels(20.0, 20.0);
        assert_eq!(app.build_zoom_view().unwrap(), fit);
    }

    #[test]
    fn mouse_pan_accounts_for_fit_scale() {
        use super::App;
        use crate::image_list::SharedImageList;
        use crate::theme::Theme;
        use ratatui::layout::Rect;

        let mut app = App::new(Theme::fallback(), (1, 1), SharedImageList::new());
        app.reset_zoom_state();
        app.image_rect = Rect::new(0, 0, 100, 50);
        app.source = Some(std::sync::Arc::new(super::ZoomSource::Raster(
            DynamicImage::new_rgba8(400, 200),
        )));
        app.zoom = 2.0;
        app.pan_pixels(10.0, -5.0);
        assert!((app.pan_x - 0.4).abs() < 1e-10);
        assert!((app.pan_y - 0.6).abs() < 1e-10);
    }

    #[test]
    fn cache_key_changes_with_dimensions_and_source_size() {
        let source = std::env::temp_dir().join(format!(
            "rview-synthetic-cache-source-{}.png",
            std::process::id()
        ));
        fs::write(&source, b"synthetic-a").unwrap();
        let first = get_cache_file_path(&source, 100, 100).unwrap();
        let resized = get_cache_file_path(&source, 200, 100).unwrap();
        fs::write(&source, b"synthetic-content-b").unwrap();
        let changed = get_cache_file_path(&source, 100, 100).unwrap();
        fs::remove_file(source).unwrap();

        assert_ne!(first, resized);
        assert_ne!(first, changed);
    }
}
