use unicode_width::UnicodeWidthChar;

pub const DEFAULT_LABEL_KEYS: [char; 9] = ['j', 'f', 'h', 'g', 'k', 'd', 'l', 's', 'a'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Word,
    Char,
    Line,
}

impl MatchMode {
    pub fn from_env(value: &str) -> Self {
        match value {
            "char" | "anywhere" => Self::Char,
            "line" | "line_start" => Self::Line,
            _ => Self::Word,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPosition {
    Left,
    Right,
    OffLeft,
}

impl KeyPosition {
    pub fn from_env(value: &str) -> Self {
        match value {
            "right" => Self::Right,
            "off_left" => Self::OffLeft,
            _ => Self::Left,
        }
    }
}

/// Anchor for drawing an overlay label. `column` is a **display** column: each
/// character contributes its terminal width, so a wide (East Asian) character
/// counts as two. This matches the ANSI absolute cursor positioning
/// (`ESC[row;colH`) the overlay uses.
///
/// Deliberately a distinct type from [`JumpTarget`]: the two carry different
/// column metrics and must never be interchanged. Keeping them separate turns a
/// mix-up into a compile error instead of a silent wide-character drift bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayAnchor {
    pub row: usize,
    pub column: usize,
}

/// Target for driving copy-mode navigation. `column` is a **character** count
/// from the start of the row. tmux's `cursor-right` skips the padding cell of a
/// wide character, so one press moves exactly one logical character regardless
/// of display width; counting display cells would overshoot on any line
/// containing wide characters.
///
/// Deliberately a distinct type from [`OverlayAnchor`] — see its docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpTarget {
    pub row: usize,
    pub column: usize,
}

fn is_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

/// Locate the character at `target_index` for overlay label placement. See
/// [`OverlayAnchor`] for why the column is measured in display cells.
pub fn overlay_anchor_for_char_index(screen: &str, target_index: usize) -> OverlayAnchor {
    let (row, column) = row_and_column(screen, target_index, ColumnMetric::DisplayWidth);
    OverlayAnchor { row, column }
}

/// Locate the character at `target_index` for copy-mode navigation. See
/// [`JumpTarget`] for why the column is measured in characters.
pub fn jump_target_for_char_index(screen: &str, target_index: usize) -> JumpTarget {
    let (row, column) = row_and_column(screen, target_index, ColumnMetric::CharCount);
    JumpTarget { row, column }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnMetric {
    DisplayWidth,
    CharCount,
}

fn row_and_column(screen: &str, target_index: usize, metric: ColumnMetric) -> (usize, usize) {
    let mut row = 0usize;
    let mut column = 0usize;

    for (index, ch) in screen.chars().enumerate() {
        if index == target_index {
            return (row, column);
        }

        if ch == '\n' {
            row += 1;
            column = 0;
        } else {
            column += match metric {
                ColumnMetric::DisplayWidth => UnicodeWidthChar::width(ch).unwrap_or(0),
                ColumnMetric::CharCount => 1,
            };
        }
    }

    (row, column)
}

pub fn positions_of(target: char, screen: &str) -> Vec<usize> {
    positions_for(target, screen, MatchMode::Word, false)
}

pub fn positions_for(
    target: char,
    screen: &str,
    match_mode: MatchMode,
    case_sensitive: bool,
) -> Vec<usize> {
    let chars: Vec<char> = screen.chars().collect();
    match match_mode {
        MatchMode::Word => word_positions(target, &chars, case_sensitive),
        MatchMode::Char => char_positions(target, &chars, case_sensitive),
        MatchMode::Line => line_positions(target, &chars, case_sensitive),
    }
}

fn chars_match(lhs: char, rhs: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        lhs == rhs
    } else {
        lhs.to_lowercase().eq(rhs.to_lowercase())
    }
}

fn word_positions(target: char, chars: &[char], case_sensitive: bool) -> Vec<usize> {
    // Word-start matching is only meaningful for word characters. A non-word
    // target (punctuation, symbols, whitespace) can never be at a word start,
    // so fall back to matching every occurrence instead of returning nothing.
    if !is_word_char(target) {
        return char_positions(target, chars, case_sensitive);
    }

    let mut positions = Vec::new();

    if let Some(first) = chars.first()
        && is_word_char(*first)
        && chars_match(*first, target, case_sensitive)
    {
        positions.push(0);
    }

    for index in 0..chars.len().saturating_sub(1) {
        if !is_word_char(chars[index])
            && is_word_char(chars[index + 1])
            && chars_match(chars[index + 1], target, case_sensitive)
        {
            positions.push(index + 1);
        }
    }

    positions
}

fn char_positions(target: char, chars: &[char], case_sensitive: bool) -> Vec<usize> {
    chars
        .iter()
        .enumerate()
        .filter_map(|(index, ch)| chars_match(*ch, target, case_sensitive).then_some(index))
        .collect()
}

fn line_positions(target: char, chars: &[char], case_sensitive: bool) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut line_start = 0usize;

    loop {
        let Some((offset, ch)) = chars[line_start..]
            .iter()
            .enumerate()
            .find(|(_, ch)| **ch != ' ' && **ch != '\t' && **ch != '\n')
        else {
            break;
        };
        let position = line_start + offset;
        if chars_match(*ch, target, case_sensitive) {
            positions.push(position);
        }

        let Some(next_newline) = chars[position..].iter().position(|ch| *ch == '\n') else {
            break;
        };
        line_start = position + next_newline + 1;
        if line_start >= chars.len() {
            break;
        }
    }

    positions
}

/// A validated set of label keys: always at least two unique, non-whitespace
/// characters. Because the invariant is enforced at construction, downstream
/// code (label expansion, recursive subset selection) never has to re-check
/// that there are "enough" keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelKeys(Vec<char>);

impl LabelKeys {
    /// Parse label keys from a raw option value, keeping the first occurrence of
    /// each non-whitespace character. Falls back to [`LabelKeys::default`] when
    /// fewer than two distinct keys remain, since a single key cannot form a
    /// distinguishable label alphabet.
    pub fn from_env(value: &str) -> Self {
        let mut keys = Vec::new();
        for ch in value.chars().filter(|ch| !ch.is_whitespace()) {
            if !keys.contains(&ch) {
                keys.push(ch);
            }
        }

        if keys.len() < 2 {
            Self::default()
        } else {
            Self(keys)
        }
    }

    pub fn as_slice(&self) -> &[char] {
        &self.0
    }

    /// Number of distinct keys. Always `>= 2` by construction.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false`: a `LabelKeys` is never empty. Present so the type reads
    /// naturally alongside `len` (and satisfies `clippy::len_without_is_empty`).
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Default for LabelKeys {
    fn default() -> Self {
        Self(DEFAULT_LABEL_KEYS.to_vec())
    }
}

pub fn labels_for(position_count: usize, label_keys: &LabelKeys) -> Vec<String> {
    let label_keys = label_keys.as_slice();
    let mut labels: Vec<String> = label_keys.iter().map(char::to_string).collect();
    while position_count > labels.len() {
        labels = labels
            .iter()
            .flat_map(|prefix| {
                label_keys
                    .iter()
                    .map(move |suffix| format!("{prefix}{suffix}"))
            })
            .collect();
    }
    labels
}

pub fn label_length_for(position_count: usize, label_keys: &LabelKeys) -> usize {
    labels_for(position_count, label_keys)
        .first()
        .map(|s| s.chars().count())
        .unwrap_or(1)
}

pub fn subset_bounds(
    key_index: usize,
    label_len: usize,
    label_key_count: usize,
) -> std::ops::Range<usize> {
    let magnitude = label_key_count.pow((label_len.saturating_sub(1)) as u32);
    let start = key_index * magnitude;
    start..(start + magnitude)
}

pub fn bounded_subset_bounds(
    key_index: usize,
    label_len: usize,
    position_count: usize,
    label_key_count: usize,
) -> Option<std::ops::Range<usize>> {
    let bounds = subset_bounds(key_index, label_len, label_key_count);
    if bounds.start >= position_count {
        return None;
    }

    Some(bounds.start..bounds.end.min(position_count))
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_LABEL_KEYS, LabelKeys, MatchMode, bounded_subset_bounds, label_length_for,
        labels_for, positions_for, positions_of, subset_bounds,
    };
    use crate::jump::{
        JumpTarget, KeyPosition, OverlayAnchor, jump_target_for_char_index,
        overlay_anchor_for_char_index,
    };

    #[test]
    fn positions_match_original_behavior() {
        let screen = "~$ echo 'hello world! easymotion for tmux :)'\nhello world! easymotion for tmux :)\n~$";
        assert_eq!(positions_of('h', screen), vec![9, 46]);
        assert_eq!(positions_of('e', screen), vec![3, 22, 59]);
        assert!(positions_of('s', screen).is_empty());
    }

    #[test]
    fn word_mode_falls_back_to_char_matching_for_non_word_targets() {
        let screen = "~$ cat src/jump.rs and src/main.rs\n/usr/local/bin";
        // A punctuation target ('/') is never a word start, so word mode must
        // surface every occurrence, identical to char mode.
        assert_eq!(
            positions_for('/', screen, MatchMode::Word, false),
            positions_for('/', screen, MatchMode::Char, false),
        );
        assert!(!positions_for('/', screen, MatchMode::Word, false).is_empty());
        // Pin concrete indices so the test fails if char-matching itself drifts,
        // not only when the two modes diverge.
        assert_eq!(
            positions_for('/', "a/b/c", MatchMode::Word, false),
            vec![1, 3]
        );

        // Other non-word targets behave the same way.
        for target in ['.', '-', '$'] {
            assert_eq!(
                positions_for(target, screen, MatchMode::Word, false),
                positions_for(target, screen, MatchMode::Char, false),
                "word-mode fallback diverged from char mode for {target:?}",
            );
        }

        // Whitespace is non-word too and is included in the fallback.
        assert_eq!(
            positions_for(' ', screen, MatchMode::Word, false),
            positions_for(' ', screen, MatchMode::Char, false),
        );
    }

    #[test]
    fn word_mode_keeps_word_start_matching_for_word_targets() {
        // Regression guard: word-character targets must be unaffected by the
        // non-word fallback and keep pure word-start semantics.
        let screen = "alpha apple\n  beta banana";
        assert_eq!(
            positions_for('a', screen, MatchMode::Word, false),
            vec![0, 6]
        );
        assert_eq!(positions_of('s', "qone qtwo qthree"), Vec::<usize>::new());
    }

    #[test]
    fn overlay_anchor_uses_display_width_and_newlines() {
        assert_eq!(
            overlay_anchor_for_char_index("あいう x\nab", 4),
            OverlayAnchor { row: 0, column: 7 }
        );
        assert_eq!(
            overlay_anchor_for_char_index("あいう x\nab", 6),
            OverlayAnchor { row: 1, column: 0 }
        );
    }

    #[test]
    fn jump_target_counts_characters_not_display_width() {
        // Copy-mode `cursor-right` skips a wide character's padding cell, so the
        // jump must count one column per logical character. The overlay, by
        // contrast, needs display cells. Both must agree on the row.
        let screen = "あいう z\nab";
        assert_eq!(
            jump_target_for_char_index(screen, 4),
            JumpTarget { row: 0, column: 4 }
        );
        assert_eq!(
            overlay_anchor_for_char_index(screen, 4),
            OverlayAnchor { row: 0, column: 7 }
        );
        // Row counting stays identical across both metrics.
        assert_eq!(
            jump_target_for_char_index(screen, 6),
            JumpTarget { row: 1, column: 0 }
        );
    }

    #[test]
    fn overlay_anchor_and_jump_target_agree_on_ascii() {
        // For pure ASCII, character count and display width coincide, so the two
        // metrics must produce identical row/column across every index.
        let screen = "alpha beta\ngamma delta\n";
        for index in 0..screen.chars().count() {
            let anchor = overlay_anchor_for_char_index(screen, index);
            let target = jump_target_for_char_index(screen, index);
            assert_eq!(
                (anchor.row, anchor.column),
                (target.row, target.column),
                "ASCII metrics diverged at index {index}"
            );
        }
    }

    #[test]
    fn labels_expand_in_easymotion_order() {
        let keys = LabelKeys::default();
        assert_eq!(labels_for(1, &keys).len(), DEFAULT_LABEL_KEYS.len());
        assert_eq!(label_length_for(DEFAULT_LABEL_KEYS.len(), &keys), 1);
        assert_eq!(label_length_for(DEFAULT_LABEL_KEYS.len() + 1, &keys), 2);
        assert_eq!(
            labels_for(DEFAULT_LABEL_KEYS.len() + 1, &keys).len(),
            DEFAULT_LABEL_KEYS.len().pow(2)
        );
    }

    #[test]
    fn subset_bounds_match_recursive_label_groups() {
        assert_eq!(
            subset_bounds(0, 2, DEFAULT_LABEL_KEYS.len()),
            0..DEFAULT_LABEL_KEYS.len()
        );
        assert_eq!(subset_bounds(2, 2, DEFAULT_LABEL_KEYS.len()), 18..27);
        assert_eq!(subset_bounds(1, 3, DEFAULT_LABEL_KEYS.len()), 81..162);
    }

    #[test]
    fn bounded_subset_bounds_keep_partial_final_groups_selectable() {
        assert_eq!(
            bounded_subset_bounds(1, 2, 10, DEFAULT_LABEL_KEYS.len()),
            Some(9..10)
        );
        assert_eq!(
            bounded_subset_bounds(2, 2, 10, DEFAULT_LABEL_KEYS.len()),
            None
        );
    }

    #[test]
    fn supports_custom_label_keys() {
        let keys = LabelKeys::from_env("abcaa");
        assert_eq!(keys.as_slice(), ['a', 'b', 'c']);
        assert_eq!(keys.len(), 3);
        assert!(!keys.is_empty());
        assert_eq!(
            labels_for(4, &keys),
            vec!["aa", "ab", "ac", "ba", "bb", "bc", "ca", "cb", "cc"]
        );
        // Fewer than two distinct keys is not a usable alphabet -> default.
        assert_eq!(LabelKeys::from_env("a"), LabelKeys::default());
        assert_eq!(LabelKeys::from_env("aaa"), LabelKeys::default());
        assert_eq!(LabelKeys::from_env("  "), LabelKeys::default());
        assert_eq!(LabelKeys::from_env(""), LabelKeys::default());
        // Whitespace is dropped; surrounding spaces don't defeat the alphabet.
        assert_eq!(LabelKeys::from_env(" a b ").as_slice(), ['a', 'b']);
    }

    #[test]
    fn label_keys_are_never_shorter_than_two() {
        // The core invariant of LabelKeys, checked over many raw inputs.
        for raw in ["", " ", "x", "xx", "  x  ", "\t\n", "ab", "abcabc", "j f h"] {
            let keys = LabelKeys::from_env(raw);
            assert!(
                keys.len() >= 2,
                "LabelKeys::from_env({raw:?}) yielded fewer than two keys"
            );
            // Keys are always unique.
            let mut seen = std::collections::HashSet::new();
            for key in keys.as_slice() {
                assert!(seen.insert(*key), "duplicate label key from {raw:?}");
            }
        }
    }

    #[test]
    fn labels_are_unique_same_length_and_cover_positions() {
        // Invariants that the recursive selection relies on, across a range of
        // match counts and both the default and a custom alphabet.
        for keys in [LabelKeys::default(), LabelKeys::from_env("abc")] {
            for count in 1..=60usize {
                let labels = labels_for(count, &keys);
                assert!(
                    labels.len() >= count,
                    "labels_for({count}) produced too few labels"
                );
                let len = label_length_for(count, &keys);
                assert!(labels.iter().all(|label| label.chars().count() == len));
                let unique: std::collections::HashSet<&String> = labels.iter().collect();
                assert_eq!(unique.len(), labels.len(), "labels_for({count}) had dupes");
            }
        }
    }

    #[test]
    fn supports_match_modes_and_case_sensitivity() {
        let screen = "Alpha beta\n  apple aardvark\nBeta alpha";
        assert_eq!(
            positions_for('a', screen, MatchMode::Word, false),
            vec![0, 13, 19, 33]
        );
        assert_eq!(
            positions_for('a', screen, MatchMode::Word, true),
            vec![13, 19, 33]
        );
        assert_eq!(
            positions_for('a', screen, MatchMode::Char, true),
            vec![4, 9, 13, 19, 20, 24, 31, 33, 37]
        );
        assert_eq!(positions_for('b', screen, MatchMode::Line, false), vec![28]);
    }

    #[test]
    fn key_position_from_env_accepts_left_right_and_off_left() {
        assert_eq!(KeyPosition::from_env("left"), KeyPosition::Left);
        assert_eq!(KeyPosition::from_env("right"), KeyPosition::Right);
        assert_eq!(KeyPosition::from_env("off_left"), KeyPosition::OffLeft);
        assert_eq!(KeyPosition::from_env("unknown"), KeyPosition::Left);
    }

    #[test]
    fn subset_bounds_partition_the_position_range() {
        // The recursive label groups must tile [0, count) with no gap or overlap
        // for every match count that uses two-character labels. If they didn't,
        // some targets would be unreachable or two labels would collide.
        let keys = LabelKeys::default();
        let k = keys.len();
        for count in 1..=(k * k) {
            let label_len = label_length_for(count, &keys);
            if label_len == 1 {
                continue;
            }
            let mut covered = Vec::new();
            for key_index in 0..k {
                if let Some(range) = bounded_subset_bounds(key_index, label_len, count, k) {
                    covered.extend(range);
                }
            }
            covered.sort_unstable();
            assert_eq!(
                covered,
                (0..count).collect::<Vec<_>>(),
                "subsets failed to tile [0, {count})"
            );
        }
    }

    #[test]
    fn positions_are_sorted_in_range_and_actually_match() {
        let screen = "Foo bar\n  baz Qux\nfoo BAR :) 4a";
        let chars: Vec<char> = screen.chars().collect();
        for mode in [MatchMode::Word, MatchMode::Char, MatchMode::Line] {
            for case_sensitive in [false, true] {
                for target in ['f', 'F', 'b', 'Q', 'o', ' ', 'z', ':', '4'] {
                    let positions = positions_for(target, screen, mode, case_sensitive);
                    assert!(
                        positions.windows(2).all(|pair| pair[0] < pair[1]),
                        "positions not strictly increasing for {target:?} {mode:?}"
                    );
                    for &position in &positions {
                        assert!(position < chars.len(), "index {position} out of range");
                        let hit = if case_sensitive {
                            chars[position] == target
                        } else {
                            chars[position].to_lowercase().eq(target.to_lowercase())
                        };
                        assert!(
                            hit,
                            "index {position} does not match {target:?} in {mode:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn word_matches_are_a_subset_of_char_matches() {
        // Word-start (and line) matching may only ever be a filter over the
        // exhaustive char matches; it must never invent a position of its own.
        let screen = "alpha Apple\n  beta_ban 42a\nEND";
        for target in ['a', 'A', 'b', '4', '_', 'e'] {
            for case_sensitive in [false, true] {
                let char_hits: std::collections::HashSet<usize> =
                    positions_for(target, screen, MatchMode::Char, case_sensitive)
                        .into_iter()
                        .collect();
                for mode in [MatchMode::Word, MatchMode::Line] {
                    let hits: std::collections::HashSet<usize> =
                        positions_for(target, screen, mode, case_sensitive)
                            .into_iter()
                            .collect();
                    assert!(
                        hits.is_subset(&char_hits),
                        "{mode:?} matches for {target:?} (cs={case_sensitive}) escaped char matches"
                    );
                }
            }
        }
    }
}
