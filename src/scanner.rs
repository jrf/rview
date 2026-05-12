use crate::image_list::SharedImageList;
use std::path::{Path, PathBuf};
use std::thread;

const BATCH_SIZE: usize = 1024;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "avif",
];

#[cfg(feature = "video")]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm", "m4v"];

fn is_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    let Some(ext) = ext else { return false };
    let ext = ext.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    #[cfg(feature = "video")]
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    false
}

pub fn spawn(paths: Vec<PathBuf>, list: SharedImageList) {
    thread::spawn(move || {
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for path in paths {
            if path.is_dir() {
                scan_dir(&path, &list, &mut batch);
            } else {
                batch.push(path);
                if batch.len() >= BATCH_SIZE {
                    list.push_batch(std::mem::take(&mut batch));
                    batch = Vec::with_capacity(BATCH_SIZE);
                }
            }
        }

        if !batch.is_empty() {
            list.push_batch(batch);
        }

        list.mark_complete();
    });
}

fn scan_dir(dir: &Path, list: &SharedImageList, batch: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        if !is_supported(&path) {
            continue;
        }
        batch.push(path);
        if batch.len() >= BATCH_SIZE {
            list.push_batch(std::mem::take(batch));
            *batch = Vec::with_capacity(BATCH_SIZE);
        }
    }
}
