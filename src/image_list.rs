use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Inner {
    paths: Vec<PathBuf>,
    filenames: Vec<String>,
    errors: Vec<String>,
}

#[derive(Clone)]
pub struct SharedImageList {
    inner: Arc<Mutex<Inner>>,
    len: Arc<AtomicUsize>,
    complete: Arc<AtomicBool>,
}

impl SharedImageList {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                paths: Vec::new(),
                filenames: Vec::new(),
                errors: Vec::new(),
            })),
            len: Arc::new(AtomicUsize::new(0)),
            complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn push_batch(&self, paths: Vec<PathBuf>) {
        let mut inner = self.inner.lock().unwrap();
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            inner.paths.push(path);
            inner.filenames.push(name);
        }
        self.len.store(inner.paths.len(), Ordering::Release);
    }

    pub fn mark_complete(&self) {
        self.complete.store(true, Ordering::Release);
    }

    pub fn push_error(&self, error: String) {
        self.inner.lock().unwrap().errors.push(error);
    }

    pub fn drain_errors(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.lock().unwrap().errors)
    }

    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn drain_since(&self, known_len: usize) -> (Vec<PathBuf>, Vec<String>) {
        let inner = self.inner.lock().unwrap();
        let paths = inner.paths[known_len..].to_vec();
        let filenames = inner.filenames[known_len..].to_vec();
        (paths, filenames)
    }

    pub fn replace_all(&self, paths: Vec<PathBuf>, filenames: Vec<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.paths = paths;
        inner.filenames = filenames;
        self.len.store(inner.paths.len(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::SharedImageList;
    use std::path::PathBuf;

    #[test]
    fn paths_and_filenames_remain_aligned() {
        let list = SharedImageList::new();
        list.push_batch(vec![PathBuf::from("synthetic-a.png")]);
        let (paths, filenames) = list.drain_since(0);
        assert_eq!(paths, vec![PathBuf::from("synthetic-a.png")]);
        assert_eq!(filenames, vec!["synthetic-a.png"]);
    }

    #[test]
    fn errors_are_drained_once() {
        let list = SharedImageList::new();
        list.push_error("synthetic scanner error".into());
        assert_eq!(list.drain_errors(), vec!["synthetic scanner error"]);
        assert!(list.drain_errors().is_empty());
    }
}
