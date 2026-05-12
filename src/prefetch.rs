use crate::app::{decode_image_with_hint, resize_decoded};
use image::{DynamicImage, RgbaImage};
use lru::LruCache;
use ratatui::layout::Rect;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::mpsc;

const CACHE_CAPACITY: usize = 8;
const PREFETCH_RADIUS: usize = 3;

pub struct Prefetcher {
    cache: LruCache<usize, DynamicImage>,
    rx: mpsc::Receiver<(usize, DynamicImage)>,
    tx: mpsc::Sender<(usize, DynamicImage)>,
    pending: std::collections::HashSet<usize>,
    target_hint: Option<(u32, u32)>,
}

impl Prefetcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap()),
            rx,
            tx,
            pending: std::collections::HashSet::new(),
            target_hint: None,
        }
    }

    pub fn set_target_hint(&mut self, rect: Rect, cell_px: (u32, u32)) {
        let w = rect.width as u32 * cell_px.0;
        let h = rect.height as u32 * cell_px.1;
        if w > 0 && h > 0 {
            self.target_hint = Some((w, h));
        }
    }

    pub fn poll(&mut self) {
        while let Ok((idx, img)) = self.rx.try_recv() {
            self.pending.remove(&idx);
            self.cache.push(idx, img);
        }
    }

    pub fn take_resized(&mut self, index: usize, rect: Rect, cell_px: (u32, u32)) -> Option<RgbaImage> {
        let img = self.cache.pop(&index)?;
        Some(resize_decoded(&img, rect, cell_px))
    }

    pub fn kick(&mut self, current: usize, images: &[PathBuf]) {
        for offset in 1..=PREFETCH_RADIUS {
            if current + offset < images.len() {
                self.ensure_decoding(current + offset, images);
            }
            if current >= offset {
                self.ensure_decoding(current - offset, images);
            }
        }
    }

    pub fn kick_gallery(&mut self, index: usize, images: &[PathBuf]) {
        self.ensure_decoding(index, images);
    }

    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.pending.clear();
    }

    fn ensure_decoding(&mut self, index: usize, images: &[PathBuf]) {
        if self.cache.contains(&index) || self.pending.contains(&index) {
            return;
        }
        self.pending.insert(index);
        let tx = self.tx.clone();
        let path = images[index].clone();
        let hint = self.target_hint;
        rayon::spawn(move || {
            if let Ok(img) = decode_image_with_hint(&path, hint) {
                let _ = tx.send((index, img));
            }
        });
    }
}
