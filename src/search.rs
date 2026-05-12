use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub struct Search {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Search {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    pub fn filter(&mut self, query: &str, filenames: &[String]) -> Vec<usize> {
        if query.is_empty() {
            return (0..filenames.len()).collect();
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(usize, u32)> = Vec::new();

        for (i, name) in filenames.iter().enumerate() {
            let haystack = Utf32Str::new(name, &mut self.buf);
            if let Some(score) = pattern.score(haystack, &mut self.matcher) {
                scored.push((i, score));
            }
            self.buf.clear();
        }

        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(i, _)| i).collect()
    }
}
