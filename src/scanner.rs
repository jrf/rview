use crate::image_list::SharedImageList;
use std::path::{Path, PathBuf};
use std::thread;

const BATCH_SIZE: usize = 64;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "avif",
];

#[cfg(feature = "video")]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm", "m4v"];

pub(crate) fn is_supported(path: &Path) -> bool {
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
            } else if is_supported(&path) {
                batch.push(path);
                if batch.len() >= BATCH_SIZE {
                    flush_batch(&list, &mut batch);
                }
            } else {
                list.push_error(format!("unsupported media format: {}", path.display()));
            }
        }

        if !batch.is_empty() {
            flush_batch(&list, &mut batch);
        }

        list.mark_complete();
    });
}

fn scan_dir(dir: &Path, list: &SharedImageList, batch: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            list.push_error(format!("{}: {error}", dir.display()));
            return;
        }
    };

    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                match entry.file_type() {
                    Ok(file_type) => {
                        if file_type.is_file() && is_supported(&path) {
                            batch.push(path);
                            if batch.len() >= BATCH_SIZE {
                                flush_batch(list, batch);
                            }
                        }
                    }
                    Err(error) => list.push_error(format!("{}: {error}", path.display())),
                }
            }
            Err(error) => list.push_error(format!("{}: {error}", dir.display())),
        }
    }
}

fn flush_batch(list: &SharedImageList, batch: &mut Vec<PathBuf>) {
    batch.sort_by(|left, right| {
        left.file_name()
            .cmp(&right.file_name())
            .then_with(|| left.cmp(right))
    });
    list.push_batch(std::mem::take(batch));
    *batch = Vec::with_capacity(BATCH_SIZE);
}

#[cfg(test)]
mod tests {
    use super::is_supported;
    use std::path::Path;

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(is_supported(Path::new("synthetic.PNG")));
        assert!(is_supported(Path::new("synthetic.jpeg")));
    }

    #[test]
    fn unsupported_extensions_are_rejected() {
        assert!(!is_supported(Path::new("synthetic.txt")));
        assert!(!is_supported(Path::new("synthetic")));
    }
}
