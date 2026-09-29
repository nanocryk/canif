const MAX_DISTANCE: usize = 3;
/// One edit allowed per this many chars of the typed name, so short names only match near-exact.
const CHARS_PER_EDIT: usize = 3;

/// Edit distance over chars (insert, delete, substitute all cost 1).
pub fn levenshtein(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            let deletion = previous[right_index + 1] + 1;
            let insertion = current[right_index] + 1;
            current[right_index + 1] = substitution.min(deletion).min(insertion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// Closest candidate within the allowed edits, else the only candidate containing `name`.
pub fn closest<'candidate>(name: &str, candidates: &[&'candidate str]) -> Option<&'candidate str> {
    let allowed = (name.chars().count() / CHARS_PER_EDIT).clamp(1, MAX_DISTANCE);
    let nearest = candidates
        .iter()
        .map(|candidate| (levenshtein(name, candidate), *candidate))
        .min_by_key(|(distance, _)| *distance)
        .filter(|(distance, _)| *distance <= allowed)
        .map(|(_, candidate)| candidate);
    if nearest.is_some() {
        return nearest;
    }
    let mut containing = candidates
        .iter()
        .filter(|candidate| candidate.contains(name));
    match (containing.next(), containing.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("carg/test", "cargo/test"), 1);
        assert_eq!(levenshtein("same", "same"), 0);
        assert_eq!(levenshtein("é", "e"), 1);
    }

    #[test]
    fn closest_by_distance() {
        let candidates = ["cargo/check", "cargo/test", "deploy"];
        assert_eq!(closest("carg/test", &candidates), Some("cargo/test"));
        assert_eq!(closest("deplyo", &candidates), Some("deploy"));
    }

    #[test]
    fn distance_ties_keep_first_candidate() {
        assert_eq!(closest("ab", &["aa", "bb"]), Some("aa"));
    }

    #[test]
    fn short_names_allow_fewer_edits() {
        let candidates = ["fmt", "lint"];
        assert_eq!(closest("ls", &candidates), None);
        assert_eq!(closest("fnt", &candidates), Some("fmt"));
        assert_eq!(closest("carg/tets", &["cargo/test"]), Some("cargo/test"));
    }

    #[test]
    fn closest_by_unique_substring() {
        let candidates = ["cargo/clippy", "cargo/test", "release/publish"];
        assert_eq!(closest("publish", &candidates), Some("release/publish"));
    }

    #[test]
    fn ambiguous_substring_gives_nothing() {
        let candidates = ["cargo/clippy", "cargo/test"];
        assert_eq!(closest("cargo", &candidates), None);
    }

    #[test]
    fn nothing_close() {
        assert_eq!(closest("zzzzzzzz", &["cargo/test"]), None);
        assert_eq!(closest("x", &[]), None);
    }
}
