use image::RgbaImage;
use lru::LruCache;
use ratatui::layout::Rect;
use std::num::NonZeroUsize;

pub const THUMB_COLS: u16 = 20;
pub const THUMB_ROWS: u16 = 8;
pub const LABEL_ROWS: u16 = 1;
pub const CELL_HEIGHT_TOTAL: u16 = THUMB_ROWS + LABEL_ROWS + 1;

pub struct GalleryState {
    pub filtered_indices: Vec<usize>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub grid_cols: usize,
    pub visible_rows: usize,
    pub grid_rect: Rect,
    pub search_active: bool,
    pub search_query: String,
}

impl GalleryState {
    pub fn new(image_count: usize) -> Self {
        Self {
            filtered_indices: (0..image_count).collect(),
            cursor: 0,
            scroll_offset: 0,
            grid_cols: 1,
            visible_rows: 1,
            grid_rect: Rect::default(),
            search_active: false,
            search_query: String::new(),
        }
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn update_grid(&mut self, grid_rect: Rect) {
        self.grid_rect = grid_rect;
        self.grid_cols = (grid_rect.width as usize / THUMB_COLS as usize).max(1);
        self.visible_rows = (grid_rect.height as usize / CELL_HEIGHT_TOTAL as usize).max(1);
        self.ensure_cursor_visible();
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.filtered_indices.get(self.cursor).copied()
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor / self.grid_cols
    }

    pub fn ensure_cursor_visible(&mut self) {
        let row = self.cursor_row();
        if row < self.scroll_offset {
            self.scroll_offset = row;
        }
        if row >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = row - self.visible_rows + 1;
        }
    }

    fn move_cursor_to(&mut self, new: usize) {
        let max = self.filtered_indices.len().saturating_sub(1);
        self.cursor = new.min(max);
        self.ensure_cursor_visible();
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.move_cursor_to(self.cursor - 1);
        }
    }

    pub fn move_right(&mut self) {
        self.move_cursor_to(self.cursor + 1);
    }

    pub fn move_up(&mut self) {
        if self.cursor >= self.grid_cols {
            self.move_cursor_to(self.cursor - self.grid_cols);
        }
    }

    pub fn move_down(&mut self) {
        let target = self.cursor + self.grid_cols;
        if target < self.filtered_indices.len() {
            self.move_cursor_to(target);
        }
    }

    pub fn move_page_up(&mut self) {
        let jump = self.visible_rows * self.grid_cols;
        self.move_cursor_to(self.cursor.saturating_sub(jump));
    }

    pub fn move_page_down(&mut self) {
        let jump = self.visible_rows * self.grid_cols;
        self.move_cursor_to(self.cursor + jump);
    }

    pub fn move_to_first(&mut self) {
        self.move_cursor_to(0);
    }

    pub fn move_to_last(&mut self) {
        let last = self.filtered_indices.len().saturating_sub(1);
        self.move_cursor_to(last);
    }

    pub fn visible_items(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let start = self.scroll_offset * self.grid_cols;
        let count = self.visible_rows * self.grid_cols;
        self.filtered_indices[start..self.filtered_indices.len().min(start + count)]
            .iter()
            .enumerate()
            .map(move |(vis, &img_idx)| (vis, img_idx))
    }

    pub fn cell_rect(&self, vis_index: usize) -> Rect {
        let col = vis_index % self.grid_cols;
        let row = vis_index / self.grid_cols;
        Rect {
            x: self.grid_rect.x + (col as u16) * THUMB_COLS,
            y: self.grid_rect.y + (row as u16) * CELL_HEIGHT_TOTAL,
            width: THUMB_COLS,
            height: THUMB_ROWS,
        }
    }
}

const THUMB_CACHE_CAPACITY: usize = 200;

pub struct ThumbnailCache {
    cache: LruCache<usize, CachedThumb>,
    next_id: u32,
}

struct CachedThumb {
    image: RgbaImage,
    kitty_id: u32,
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(THUMB_CACHE_CAPACITY).unwrap()),
            next_id: 1,
        }
    }

    pub fn contains(&self, index: usize) -> bool {
        self.cache.contains(&index)
    }

    pub fn peek(&self, index: usize) -> Option<(&RgbaImage, u32)> {
        self.cache
            .peek(&index)
            .map(|t| (&t.image, t.kitty_id))
    }

    pub fn insert(&mut self, index: usize, image: RgbaImage) {
        let id = self.next_id;
        self.next_id += 1;
        self.cache.push(index, CachedThumb {
            image,
            kitty_id: id,
        });
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}
