use nucleo_matcher::{
    Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};

/// A query uses the exact same fuzzy atoms in every search screen.
pub struct Query {
    terms: Vec<Atom>,
}

impl Query {
    pub fn new(text: &str) -> Self {
        Self {
            terms: text
                .split_whitespace()
                .map(|term| {
                    Atom::new(
                        term,
                        CaseMatching::Ignore,
                        Normalization::Smart,
                        AtomKind::Fuzzy,
                        false,
                    )
                })
                .collect(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FieldMatch {
    pub score: u32,
    pub indices: Vec<Vec<usize>>,
}

#[derive(Default)]
pub struct FieldMatcher {
    matcher: Matcher,
    buffer: Vec<char>,
}

impl FieldMatcher {
    fn matched(&mut self, label: &str, term: &Atom) -> Option<(u32, Vec<usize>)> {
        self.buffer.clear();
        self.buffer.extend(label.chars());
        let haystack = Utf32Str::Unicode(&self.buffer);
        let mut indices = Vec::new();
        let score = term.indices(haystack, &mut self.matcher, &mut indices)?;
        Some((
            u32::from(score),
            indices.into_iter().map(|i| i as usize).collect(),
        ))
    }

    pub fn matches(&mut self, labels: &[&str], query: &Query) -> Option<FieldMatch> {
        let mut matches = vec![Vec::new(); labels.len()];
        let mut score = 0;
        let mut joined_labels = None;
        for term in &query.terms {
            let best = labels
                .iter()
                .enumerate()
                .filter_map(|(level, label)| {
                    self.matched(label, term)
                        .map(|(score, indices)| (score, level, indices))
                })
                .max_by_key(|(score, level, _)| (*score, *level));
            if let Some((term_score, level, indices)) = best {
                score += term_score;
                matches[level].extend(indices);
                continue;
            }
            let (joined, ranges) = joined_labels.get_or_insert_with(|| {
                let mut start = 0;
                let ranges: Vec<_> = labels
                    .iter()
                    .map(|label| {
                        let end = start + label.chars().count();
                        let range = start..end;
                        start = end + 1;
                        range
                    })
                    .collect();
                (labels.join(" "), ranges)
            });
            let (term_score, indices) = self.matched(joined, term)?;
            score += term_score;
            for (level, range) in ranges.iter().enumerate() {
                matches[level].extend(
                    indices
                        .iter()
                        .filter(|index| range.contains(index))
                        .map(|index| index - range.start),
                );
            }
        }
        for indices in &mut matches {
            indices.sort_unstable();
            indices.dedup();
        }
        Some(FieldMatch {
            score,
            indices: matches,
        })
    }
}
