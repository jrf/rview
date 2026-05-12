use crate::app::load_and_resize;
use image::RgbaImage;
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

struct Slot {
    index: usize,
    rect: Rect,
    cell_px: (u32, u32),
    image: Option<RgbaImage>,
}

pub struct Prefetcher {
    prev: Arc<Mutex<Option<Slot>>>,
    next: Arc<Mutex<Option<Slot>>>,
}

const WAIT_TIMEOUT: Duration = Duration::from_millis(1000);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

impl Prefetcher {
    pub fn new() -> Self {
        Self {
            prev: Arc::new(Mutex::new(None)),
            next: Arc::new(Mutex::new(None)),
        }
    }

    pub fn kick(&self, current: usize, images: &[PathBuf], rect: Rect, cell_px: (u32, u32)) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }

        if current > 0 {
            self.maybe_spawn(&self.prev, current - 1, images, rect, cell_px);
        }
        if current + 1 < images.len() {
            self.maybe_spawn(&self.next, current + 1, images, rect, cell_px);
        }
    }

    pub fn take(&self, index: usize, rect: Rect, cell_px: (u32, u32)) -> Option<RgbaImage> {
        for slot_arc in [&self.prev, &self.next] {
            {
                let slot = slot_arc.lock().unwrap();
                match slot.as_ref() {
                    Some(s) if s.index == index && s.rect == rect && s.cell_px == cell_px => {}
                    _ => continue,
                }
            }

            let deadline = Instant::now() + WAIT_TIMEOUT;
            loop {
                {
                    let mut slot = slot_arc.lock().unwrap();
                    if let Some(ref mut s) = *slot {
                        if s.index == index && s.rect == rect && s.cell_px == cell_px {
                            if let Some(img) = s.image.take() {
                                *slot = None;
                                return Some(img);
                            }
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
        None
    }

    pub fn invalidate(&self) {
        *self.prev.lock().unwrap() = None;
        *self.next.lock().unwrap() = None;
    }

    fn maybe_spawn(
        &self,
        slot_arc: &Arc<Mutex<Option<Slot>>>,
        index: usize,
        images: &[PathBuf],
        rect: Rect,
        cell_px: (u32, u32),
    ) {
        {
            let slot = slot_arc.lock().unwrap();
            if let Some(ref s) = *slot {
                if s.index == index && s.rect == rect && s.cell_px == cell_px {
                    return;
                }
            }
        }

        *slot_arc.lock().unwrap() = Some(Slot {
            index,
            rect,
            cell_px,
            image: None,
        });

        let slot_arc = Arc::clone(slot_arc);
        let path = images[index].clone();
        thread::spawn(move || {
            let img = load_and_resize(&path, rect, cell_px).ok();
            let mut slot = slot_arc.lock().unwrap();
            if let Some(ref mut s) = *slot {
                if s.index == index && s.rect == rect && s.cell_px == cell_px {
                    s.image = img;
                }
            }
        });
    }
}
