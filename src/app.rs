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
use std::sync::{OnceLock, mpsc};

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
    /// Full-resolution decoded source for the current fullscreen image, used for zoomed crops.
    source: Option<DynamicImage>,
    source_for: Option<usize>,
    source_rx: Option<mpsc::Receiver<io::Result<DynamicImage>>>,
    source_target: Option<usize>,

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
        self.loaded = None;
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
            self.loaded = None;
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
            let _ = tx.send(decode_image_with_hint(&path, None));
        });
    }

    /// Receive an async full-resolution source decode for zooming.
    pub fn poll_source(&mut self) -> bool {
        let result = self.source_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            let idx = self.source_target.take();
            self.source_rx = None;
            if let (Some(idx), Ok(img)) = (idx, result) {
                if idx == self.current {
                    self.source = Some(img);
                    self.source_for = Some(idx);
                    self.zoom_dirty = true;
                    self.needs_render = true;
                }
            }
            return true;
        }
        false
    }

    pub fn enter_gallery(&mut self) {
        self.mode = ViewMode::Gallery;
        self.loaded = None;
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
        self.loaded = None;

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
        self.loaded = None;
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
            self.loaded = None;
            return;
        }

        self.mode = ViewMode::Fullscreen;
        if let Some(img) = self
            .prefetcher
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
        self.graphics_storage_dirty = true;
        self.needs_render = true;
    }

    pub fn load_if_needed(&mut self) {
        // Zoomed view: crop the full-resolution source around the pan point and
        // resize that crop to fill the viewport.
        if self.zoom > 1.0 {
            if self.source_for == Some(self.current) {
                if self.zoom_dirty
                    || self.loaded.is_none()
                    || self.loaded_for_rect != self.image_rect
                {
                    if let Some(view) = self.build_zoom_view() {
                        self.loaded = Some(view);
                        self.error = None;
                        self.loaded_for_rect = self.image_rect;
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
            self.zoom_dirty = false;
            if self.loaded.is_some() {
                return;
            }
        }

        if self.loaded.is_some() && self.loaded_for_rect == self.image_rect && !self.zoom_dirty {
            return;
        }
        self.zoom_dirty = false;

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

    /// Build the zoomed, panned crop of the current full-resolution source, sized
    /// to fill the viewport. Returns `None` when no source is available.
    fn build_zoom_view(&self) -> Option<RgbaImage> {
        let src = self.source.as_ref()?;
        let (ow, oh) = (src.width(), src.height());
        if ow == 0 || oh == 0 {
            return None;
        }

        let vw = (self.image_rect.width as u32 * self.cell_px.0).max(1);
        let vh = (self.image_rect.height as u32 * self.cell_px.1).max(1);

        // Fit-to-window scale, never upscaling the base image (matches zoom == 1.0).
        let fit = f64::min(vw as f64 / ow as f64, vh as f64 / oh as f64);
        let base = fit.min(1.0);
        let eff = base * self.zoom;

        // Source-pixel crop window that maps onto the viewport at the effective scale.
        let crop_w = ((vw as f64 / eff).round() as u32).clamp(1, ow);
        let crop_h = ((vh as f64 / eff).round() as u32).clamp(1, oh);
        let max_x = ow.saturating_sub(crop_w);
        let max_y = oh.saturating_sub(crop_h);
        let x = (self.pan_x * max_x as f64).round() as u32;
        let y = (self.pan_y * max_y as f64).round() as u32;

        let cropped = src.crop_imm(x, y, crop_w, crop_h);
        let dst_w = ((crop_w as f64 * eff).round() as u32).max(1).min(vw);
        let dst_h = ((crop_h as f64 * eff).round() as u32).max(1).min(vh);
        Some(resize_to_exact(&cropped, dst_w, dst_h))
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
    use super::{get_cache_file_path, resize_decoded_to_dims, resize_to_exact};
    use image::DynamicImage;
    use std::fs;

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
        app.source = Some(DynamicImage::new_rgba8(400, 400));
        app.source_for = Some(0);
        app.current = 0;

        // At 2x zoom the effective scale is 0.5, so the crop is 200x200 source px
        // upscaled to exactly the 100x100 viewport.
        app.zoom = 2.0;
        let view = app.build_zoom_view().expect("zoom view");
        assert_eq!(view.dimensions(), (100, 100));
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
