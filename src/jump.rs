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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPosition {
    pub row: usize,
    pub column: usize,
}

fn is_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

/// Row/column for drawing an overlay label. `column` is a **display** column:
/// each character contributes its terminal width, so a wide (East Asian)
/// character counts as two. This matches ANSI absolute cursor positioning
/// (`ESC[row;colH`) used by the overlay.
pub fn display_position_for_char_index(screen: &str, target_index: usize) -> DisplayPosition {
    position_for_char_index(screen, target_index, ColumnMetric::DisplayWidth)
}

/// Row/column for driving copy-mode `cursor-right`. `column` is a **character**
/// count from the start of the row. tmux's `cursor-right` skips the padding
/// cell of a wide character, so one press moves exactly one logical character
/// regardless of display width; counting display cells would overshoot on any
/// line containing wide characters.
pub fn jump_position_for_char_index(screen: &str, target_index: usize) -> DisplayPosition {
    position_for_char_index(screen, target_index, ColumnMetric::CharCount)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnMetric {
    DisplayWidth,
    CharCount,
}

fn position_for_char_index(
    screen: &str,
    target_index: usize,
    metric: ColumnMetric,
) -> DisplayPosition {
    let mut row = 0usize;
    let mut column = 0usize;

    for (index, ch) in screen.chars().enumerate() {
        if index == target_index {
            return DisplayPosition { row, column };
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

    DisplayPosition { row, column }
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

pub fn label_keys_from_env(value: &str) -> Vec<char> {
    let mut keys = Vec::new();
    for ch in value.chars().filter(|ch| !ch.is_whitespace()) {
        if !keys.contains(&ch) {
            keys.push(ch);
        }
    }

    if keys.len() < 2 {
        DEFAULT_LABEL_KEYS.to_vec()
    } else {
        keys
    }
}

pub fn labels_for(position_count: usize, label_keys: &[char]) -> Vec<String> {
    let label_keys = if label_keys.len() < 2 {
        DEFAULT_LABEL_KEYS.as_slice()
    } else {
        label_keys
    };
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

pub fn label_length_for(position_count: usize, label_keys: &[char]) -> usize {
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
        DEFAULT_LABEL_KEYS, MatchMode, bounded_subset_bounds, label_keys_from_env,
        label_length_for, labels_for, positions_for, positions_of, subset_bounds,
    };
    use crate::jump::{
        DisplayPosition, KeyPosition, display_position_for_char_index, jump_position_for_char_index,
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
    fn display_position_uses_display_width_and_newlines() {
        assert_eq!(
            display_position_for_char_index("あいう x\nab", 4),
            DisplayPosition { row: 0, column: 7 }
        );
        assert_eq!(
            display_position_for_char_index("あいう x\nab", 6),
            DisplayPosition { row: 1, column: 0 }
        );
    }

    #[test]
    fn jump_position_counts_characters_not_display_width() {
        // Copy-mode `cursor-right` skips a wide character's padding cell, so the
        // jump must count one column per logical character. The overlay, by
        // contrast, needs display cells. Both must agree on the row.
        let screen = "あいう z\nab";
        assert_eq!(
            jump_position_for_char_index(screen, 4),
            DisplayPosition { row: 0, column: 4 }
        );
        assert_eq!(
            display_position_for_char_index(screen, 4),
            DisplayPosition { row: 0, column: 7 }
        );
        // Row counting stays identical across both metrics.
        assert_eq!(
            jump_position_for_char_index(screen, 6),
            DisplayPosition { row: 1, column: 0 }
        );
    }

    #[test]
    fn labels_expand_in_easymotion_order() {
        assert_eq!(
            labels_for(1, &DEFAULT_LABEL_KEYS).len(),
            DEFAULT_LABEL_KEYS.len()
        );
        assert_eq!(
            label_length_for(DEFAULT_LABEL_KEYS.len(), &DEFAULT_LABEL_KEYS),
            1
        );
        assert_eq!(
            label_length_for(DEFAULT_LABEL_KEYS.len() + 1, &DEFAULT_LABEL_KEYS),
            2
        );
        assert_eq!(
            labels_for(DEFAULT_LABEL_KEYS.len() + 1, &DEFAULT_LABEL_KEYS).len(),
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
        let keys = label_keys_from_env("abcaa");
        assert_eq!(keys, vec!['a', 'b', 'c']);
        assert_eq!(
            labels_for(4, &keys),
            vec!["aa", "ab", "ac", "ba", "bb", "bc", "ca", "cb", "cc"]
        );
        assert_eq!(label_keys_from_env("a"), DEFAULT_LABEL_KEYS.to_vec());
        assert_eq!(label_keys_from_env("aaa"), DEFAULT_LABEL_KEYS.to_vec());
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
}
