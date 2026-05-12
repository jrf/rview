use crate::app::{decode_image, resize_decoded};
use image::{DynamicImage, RgbaImage};
use lru::LruCache;
use ratatui::layout::Rect;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::mpsc;

const CACHE_CAPACITY: usize = 5;

pub struct Prefetcher {
    cache: LruCache<usize, DynamicImage>,
    rx: mpsc::Receiver<(usize, DynamicImage)>,
    tx: mpsc::Sender<(usize, DynamicImage)>,
    pending: std::collections::HashSet<usize>,
}

impl Prefetcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap()),
            rx,
            tx,
            pending: std::collections::HashSet::new(),
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
        if current > 0 {
            self.ensure_decoding(current - 1, images);
        }
        if current + 1 < images.len() {
            self.ensure_decoding(current + 1, images);
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
        rayon::spawn(move || {
            if let Ok(img) = decode_image(&path) {
                let _ = tx.send((index, img));
            }
        });
    }
}
