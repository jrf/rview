use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub fn filter(query: &str, filenames: &[String]) -> Vec<usize> {
    if query.is_empty() {
        return (0..filenames.len()).collect();
    }

    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = Vec::new();

    for (i, name) in filenames.iter().enumerate() {
        let haystack = Utf32Str::new(name, &mut buf);
        if let Some(score) = pattern.score(haystack, &mut matcher) {
            scored.push((i, score));
        }
        buf.clear();
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(i, _)| i).collect()
}
