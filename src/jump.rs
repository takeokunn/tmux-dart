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
    OffLeft,
}

impl KeyPosition {
    pub fn from_env(value: &str) -> Self {
        match value {
            "off_left" => Self::OffLeft,
            _ => Self::Left,
        }
    }
}

fn is_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
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
    let label_keys = if label_keys.is_empty() {
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
        .map(std::string::String::len)
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

    #[test]
    fn positions_match_original_behavior() {
        let screen = "~$ echo 'hello world! easymotion for tmux :)'\nhello world! easymotion for tmux :)\n~$";
        assert_eq!(positions_of('h', screen), vec![9, 46]);
        assert_eq!(positions_of('e', screen), vec![3, 22, 59]);
        assert!(positions_of('s', screen).is_empty());
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
}
