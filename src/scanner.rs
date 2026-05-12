use crate::image_list::SharedImageList;
use std::path::{Path, PathBuf};
use std::thread;

const BATCH_SIZE: usize = 1024;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "avif",
];

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
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
    let mut dir_images: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .collect();
    dir_images.sort();

    for path in dir_images {
        batch.push(path);
        if batch.len() >= BATCH_SIZE {
            list.push_batch(std::mem::take(batch));
            *batch = Vec::with_capacity(BATCH_SIZE);
        }
    }
}
