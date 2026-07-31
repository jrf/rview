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

    scored.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::filter;

    #[test]
    fn empty_query_preserves_input_order() {
        let filenames = vec!["b.png".into(), "a.png".into()];
        assert_eq!(filter("", &filenames), vec![0, 1]);
    }

    #[test]
    fn fuzzy_query_returns_only_matches() {
        let filenames = vec!["vacation.jpg".into(), "report.png".into()];
        assert_eq!(filter("vac", &filenames), vec![0]);
    }
}
