use std::collections::HashSet;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
/// grid cell contributes its terminal width, so a wide (East Asian or emoji)
/// cell counts as two. This matches the ANSI absolute cursor positioning
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCell {
    pub anchor: OverlayAnchor,
    pub width: usize,
}

/// Target for driving copy-mode navigation. `column` is a **grid-cell** count
/// from the start of the row: one copy-mode `cursor-right` press moves exactly
/// one cell. tmux stores a whole grapheme cluster — a base character plus its
/// combining marks, ZWJ emoji sequences, variation selectors, flag (regional
/// indicator) pairs, and skin-tone modifiers — in a single cell, and a press
/// also skips the padding cell of a wide character. Counting chars would
/// overshoot on NFD text (macOS emits "ぎ" as き + ゙) and on emoji clusters;
/// counting display cells would overshoot on any wide character.
///
/// Deliberately a distinct type from [`OverlayAnchor`] — see its docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpTarget {
    pub row: usize,
    pub column: usize,
}

fn is_word_char(ch: char) -> bool {
    // Zero-width characters (combining marks, ZWJ, variation selectors) share
    // their base character's cell, so they continue a word rather than break
    // it. Without this, NFD text — e.g. "ぎ" captured as き + ゙ — would surface
    // a bogus word start after every combining mark.
    ch == '_' || ch.is_alphanumeric() || UnicodeWidthChar::width(ch) == Some(0)
}

/// Locate the character at `target_index` for overlay label placement. See
/// [`OverlayAnchor`] for why the column is measured in display cells.
pub fn overlay_anchor_for_char_index(screen: &str, target_index: usize) -> OverlayAnchor {
    let cell = cell_position_for_char_index(screen, target_index);
    OverlayAnchor {
        row: cell.row,
        column: cell.display_column,
    }
}

/// Locate multiple overlay targets in one grapheme traversal. Results preserve
/// input order even when indices are unsorted or repeated.
pub fn overlay_cells_for_char_indices(screen: &str, target_indices: &[usize]) -> Vec<OverlayCell> {
    cell_positions_for_char_indices(screen, target_indices)
        .into_iter()
        .map(|cell| OverlayCell {
            anchor: OverlayAnchor {
                row: cell.row,
                column: cell.display_column,
            },
            width: cell.cell_width,
        })
        .collect()
}

/// Locate the character at `target_index` for copy-mode navigation. See
/// [`JumpTarget`] for why the column is measured in grid cells.
pub fn jump_target_for_char_index(screen: &str, target_index: usize) -> JumpTarget {
    let cell = cell_position_for_char_index(screen, target_index);
    JumpTarget {
        row: cell.row,
        column: cell.cell_column,
    }
}

/// Display width (always at least 1) of the grid cell holding the character at
/// `target_index`. Lets the overlay place a label immediately to the right of
/// its target even when the target is a multi-char cluster such as ☝️, whose
/// width comes from the variation selector, not the base character alone.
pub fn display_cell_width_for_char_index(screen: &str, target_index: usize) -> usize {
    cell_position_for_char_index(screen, target_index).cell_width
}

/// Position of the grid cell that holds the character at a given char index.
///
/// tmux keeps an entire grapheme cluster in one cell (verified against tmux
/// 3.6a for combining marks, ZWJ sequences, variation selectors, flag pairs,
/// and skin-tone modifiers), so both column metrics walk clusters, not chars.
/// `unicode_width`'s **str** width mirrors that cell's display width (e.g.
/// ☝ + VS16 → 2), which the per-char width cannot express.
#[derive(Clone, Copy)]
struct CellPosition {
    row: usize,
    /// Display column where the cell starts (sum of preceding cell widths).
    display_column: usize,
    /// Cell index within the row: one `cursor-right` press per cell.
    cell_column: usize,
    /// Display width of the cell itself, at least 1.
    cell_width: usize,
}

fn cell_position_for_char_index(screen: &str, target_index: usize) -> CellPosition {
    cell_positions_for_char_indices(screen, &[target_index])[0]
}

fn cell_positions_for_char_indices(screen: &str, target_indices: &[usize]) -> Vec<CellPosition> {
    if target_indices.is_empty() {
        return Vec::new();
    }

    let mut row = 0usize;
    let mut display_column = 0usize;
    let mut cell_column = 0usize;
    let mut char_index = 0usize;
    let mut reordered_targets = if target_indices.windows(2).all(|pair| pair[0] <= pair[1]) {
        None
    } else {
        let mut targets: Vec<_> = target_indices.iter().copied().enumerate().collect();
        targets.sort_unstable_by_key(|&(_, target_index)| target_index);
        Some(targets)
    };
    let mut positions = vec![
        CellPosition {
            row: 0,
            display_column: 0,
            cell_column: 0,
            cell_width: 1,
        };
        target_indices.len()
    ];
    let mut next_target = 0usize;

    for cluster in screen.graphemes(true) {
        let char_count = cluster.chars().count();
        let cluster_end = char_index.saturating_add(char_count);
        let width = UnicodeWidthStr::width(cluster);

        while let Some((output_index, target_index)) = reordered_targets
            .as_ref()
            .and_then(|targets| targets.get(next_target).copied())
            .or_else(|| {
                reordered_targets
                    .is_none()
                    .then(|| target_indices.get(next_target).copied())
                    .flatten()
                    .map(|target_index| (next_target, target_index))
            })
        {
            if target_index >= cluster_end {
                break;
            }

            positions[output_index] = CellPosition {
                row,
                display_column,
                cell_column,
                cell_width: if cluster == "\n" { 1 } else { width.max(1) },
            };
            next_target = next_target.saturating_add(1);
        }

        if next_target == target_indices.len() {
            break;
        }

        if cluster == "\n" {
            row = row.saturating_add(1);
            display_column = 0;
            cell_column = 0;
        } else {
            // A width-0 cluster is an orphan combining mark with no base
            // character; tmux drops it from the grid, so it occupies no cell.
            if width > 0 {
                display_column = display_column.saturating_add(width);
                cell_column = cell_column.saturating_add(1);
            }
        }

        char_index = cluster_end;
    }

    let fallback = CellPosition {
        row,
        display_column,
        cell_column,
        cell_width: 1,
    };
    if let Some(ordered_targets) = reordered_targets.as_mut() {
        for &(output_index, _) in &ordered_targets[next_target..] {
            positions[output_index] = fallback;
        }
    } else {
        positions[next_target..].fill(fallback);
    }

    positions
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
    match match_mode {
        MatchMode::Word => word_positions(target, screen, case_sensitive),
        MatchMode::Char => char_positions(target, screen, case_sensitive),
        MatchMode::Line => line_positions(target, screen, case_sensitive),
    }
}

fn chars_match(lhs: char, rhs: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        lhs == rhs
    } else {
        lhs.to_lowercase().eq(rhs.to_lowercase())
    }
}

fn word_positions(target: char, screen: &str, case_sensitive: bool) -> Vec<usize> {
    // Word-start matching is only meaningful for word characters. A non-word
    // target (punctuation, symbols, whitespace) can never be at a word start,
    // so fall back to matching every occurrence instead of returning nothing.
    if !is_word_char(target) {
        return char_positions(target, screen, case_sensitive);
    }

    let mut positions = Vec::new();
    let mut previous_was_word = false;
    for (index, ch) in screen.chars().enumerate() {
        let is_word = is_word_char(ch);
        if is_word && !previous_was_word && chars_match(ch, target, case_sensitive) {
            positions.push(index);
        }
        previous_was_word = is_word;
    }

    positions
}

fn char_positions(target: char, screen: &str, case_sensitive: bool) -> Vec<usize> {
    screen
        .chars()
        .enumerate()
        .filter_map(|(index, ch)| chars_match(ch, target, case_sensitive).then_some(index))
        .collect()
}

fn line_positions(target: char, screen: &str, case_sensitive: bool) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut awaiting_line_content = true;
    for (index, ch) in screen.chars().enumerate() {
        if ch == '\n' {
            awaiting_line_content = true;
        } else if awaiting_line_content && ch != ' ' && ch != '\t' {
            if chars_match(ch, target, case_sensitive) {
                positions.push(index);
            }
            awaiting_line_content = false;
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
    /// each non-whitespace, non-control character. Falls back to
    /// [`LabelKeys::default`] when fewer than two distinct keys remain, since a
    /// single key cannot form a distinguishable label alphabet.
    pub fn from_env(value: &str) -> Self {
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for ch in value
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        {
            if seen.insert(ch) {
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
    labels_up_to(position_count, label_keys)
}

pub fn label_length_for(position_count: usize, label_keys: &LabelKeys) -> usize {
    label_length_for_count(position_count, label_keys.len())
}

pub fn labels_up_to(position_count: usize, label_keys: &LabelKeys) -> Vec<String> {
    let label_keys = label_keys.as_slice();
    if position_count == 0 {
        return Vec::new();
    }

    let label_len = label_length_for_count(position_count, label_keys.len());
    (0..position_count)
        .map(|index| label_for_index(index, label_len, label_keys))
        .collect()
}

fn label_length_for_count(position_count: usize, label_key_count: usize) -> usize {
    debug_assert!(label_key_count >= 2);

    let mut label_len = 1usize;
    let mut capacity = label_key_count;
    while position_count > capacity {
        label_len += 1;
        capacity = capacity.saturating_mul(label_key_count);
    }

    label_len
}

fn label_for_index(index: usize, label_len: usize, label_keys: &[char]) -> String {
    let key_count = label_keys.len();
    debug_assert!(label_len <= usize::BITS as usize);
    let mut digits = [0usize; usize::BITS as usize];
    let mut value = index;

    for slot in (0..label_len).rev() {
        digits[slot] = value % key_count;
        value /= key_count;
    }

    debug_assert_eq!(value, 0);

    digits[..label_len]
        .iter()
        .map(|&digit| label_keys[digit])
        .collect()
}

pub fn subset_bounds(
    key_index: usize,
    label_len: usize,
    label_key_count: usize,
) -> std::ops::Range<usize> {
    let exponent = u32::try_from(label_len.saturating_sub(1)).unwrap_or(u32::MAX);
    let magnitude = label_key_count.saturating_pow(exponent);
    let start = key_index.saturating_mul(magnitude);
    start..start.saturating_add(magnitude)
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
        labels_for, labels_up_to, positions_for, positions_of, subset_bounds,
    };
    use crate::jump::{
        JumpTarget, KeyPosition, OverlayAnchor, OverlayCell, display_cell_width_for_char_index,
        jump_target_for_char_index, overlay_anchor_for_char_index, overlay_cells_for_char_indices,
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
    fn overlay_cells_preserve_unsorted_duplicate_target_order() {
        let cells = overlay_cells_for_char_indices("☝\u{fe0f} x\nab", &[5, 0, 1, 0, usize::MAX]);

        assert_eq!(
            cells,
            vec![
                OverlayCell {
                    anchor: OverlayAnchor { row: 1, column: 0 },
                    width: 1,
                },
                OverlayCell {
                    anchor: OverlayAnchor { row: 0, column: 0 },
                    width: 2,
                },
                OverlayCell {
                    anchor: OverlayAnchor { row: 0, column: 0 },
                    width: 2,
                },
                OverlayCell {
                    anchor: OverlayAnchor { row: 0, column: 0 },
                    width: 2,
                },
                OverlayCell {
                    anchor: OverlayAnchor { row: 1, column: 2 },
                    width: 1,
                },
            ]
        );
    }

    #[test]
    fn jump_target_counts_grid_cells_not_chars_or_display_width() {
        // Copy-mode `cursor-right` skips a wide character's padding cell, so the
        // jump must count one column per grid cell. The overlay, by contrast,
        // needs display cells. Both must agree on the row.
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
    fn clusters_count_one_cell_for_jump_and_full_width_for_overlay() {
        // tmux stores each of these clusters in a single grid cell (verified
        // against tmux 3.6a): one cursor-right press each, with the display
        // width of the whole cluster. Counting chars or per-char widths would
        // overshoot past the line end and wrap onto the next row.
        let cases = [
            ("か\u{3099}き\u{3099} z", 3, 5, "NFD combining marks"),
            ("👨\u{200d}👩\u{200d}👧 z", 2, 3, "ZWJ family emoji"),
            ("☝\u{fe0f} z", 2, 3, "VS16 emoji presentation"),
            ("🇯🇵 z", 2, 3, "regional indicator pair"),
            ("👍🏻 z", 2, 3, "skin-tone modifier"),
        ];
        for (screen, cell_column, display_column, kind) in cases {
            let z_index = screen.chars().count() - 1;
            assert_eq!(
                jump_target_for_char_index(screen, z_index),
                JumpTarget {
                    row: 0,
                    column: cell_column
                },
                "jump column for {kind}"
            );
            assert_eq!(
                overlay_anchor_for_char_index(screen, z_index),
                OverlayAnchor {
                    row: 0,
                    column: display_column
                },
                "overlay column for {kind}"
            );
        }
    }

    #[test]
    fn mid_cluster_index_resolves_to_the_cell_start() {
        // Index 2 is 👩, inside the family cluster: both metrics must anchor to
        // the cell the char lives in, never to a position inside it.
        let screen = "👨\u{200d}👩\u{200d}👧 z";
        assert_eq!(
            jump_target_for_char_index(screen, 2),
            JumpTarget { row: 0, column: 0 }
        );
        assert_eq!(
            overlay_anchor_for_char_index(screen, 2),
            OverlayAnchor { row: 0, column: 0 }
        );
    }

    #[test]
    fn display_cell_width_covers_the_whole_cluster() {
        assert_eq!(display_cell_width_for_char_index("a b", 0), 1);
        assert_eq!(display_cell_width_for_char_index("あ b", 0), 2);
        // The width-2 comes from the variation selector, not the base char.
        assert_eq!(display_cell_width_for_char_index("☝\u{fe0f} b", 0), 2);
        assert_eq!(
            display_cell_width_for_char_index("👨\u{200d}👩\u{200d}👧 b", 0),
            2
        );
        assert_eq!(display_cell_width_for_char_index("か\u{3099} b", 0), 2);
    }

    #[test]
    fn word_mode_treats_combining_marks_as_word_continuation() {
        // NFD "がぎ": き follows a combining mark but is mid-word, so it must
        // not surface as a word start; the word still starts at か.
        let screen = "か\u{3099}き\u{3099} zebra";
        assert_eq!(
            positions_for('き', screen, MatchMode::Word, false),
            Vec::<usize>::new()
        );
        assert_eq!(positions_for('か', screen, MatchMode::Word, false), vec![0]);
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
        assert_eq!(labels_for(0, &keys).len(), 0);
        assert_eq!(labels_for(1, &keys).len(), 1);
        assert_eq!(label_length_for(DEFAULT_LABEL_KEYS.len(), &keys), 1);
        assert_eq!(label_length_for(DEFAULT_LABEL_KEYS.len() + 1, &keys), 2);
        assert_eq!(
            labels_for(DEFAULT_LABEL_KEYS.len() + 1, &keys).len(),
            DEFAULT_LABEL_KEYS.len() + 1
        );
    }

    #[test]
    fn labels_up_to_returns_only_the_requested_prefix() {
        let keys = LabelKeys::default();
        assert!(labels_up_to(0, &keys).is_empty());
        assert_eq!(
            labels_up_to(4, &keys),
            vec!["j", "f", "h", "g"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            labels_up_to(DEFAULT_LABEL_KEYS.len() + 1, &keys),
            vec!["jj", "jf", "jh", "jg", "jk", "jd", "jl", "js", "ja", "fj"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
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
    fn subset_bounds_saturate_for_untrusted_sizes() {
        assert_eq!(subset_bounds(usize::MAX, 2, 2), usize::MAX..usize::MAX);
        assert_eq!(subset_bounds(1, usize::MAX, 2), usize::MAX..usize::MAX);
        assert_eq!(bounded_subset_bounds(usize::MAX, 2, usize::MAX, 2), None);
    }

    #[test]
    fn supports_custom_label_keys() {
        let keys = LabelKeys::from_env("abcaa");
        assert_eq!(keys.as_slice(), ['a', 'b', 'c']);
        assert_eq!(keys.len(), 3);
        assert!(!keys.is_empty());
        assert_eq!(labels_for(4, &keys), vec!["aa", "ab", "ac", "ba"]);
        // Fewer than two distinct keys is not a usable alphabet -> default.
        assert_eq!(LabelKeys::from_env("a"), LabelKeys::default());
        assert_eq!(LabelKeys::from_env("aaa"), LabelKeys::default());
        assert_eq!(LabelKeys::from_env("  "), LabelKeys::default());
        assert_eq!(LabelKeys::from_env(""), LabelKeys::default());
        // Whitespace is dropped; surrounding spaces don't defeat the alphabet.
        assert_eq!(LabelKeys::from_env(" a b ").as_slice(), ['a', 'b']);
        // Control characters must never reach raw terminal rendering.
        assert_eq!(LabelKeys::from_env("a\u{1b}b\u{7f}").as_slice(), ['a', 'b']);
        assert_eq!(LabelKeys::from_env("a\u{1b}\u{7f}"), LabelKeys::default());
    }

    #[test]
    fn label_keys_are_never_shorter_than_two() {
        // The core invariant of LabelKeys, checked over many raw inputs.
        for raw in [
            "",
            " ",
            "x",
            "xx",
            "  x  ",
            "\t\n",
            "\u{1b}\u{7f}",
            "ab",
            "abcabc",
            "j f h",
        ] {
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
                let prefix = labels_up_to(count, &keys);
                assert_eq!(labels.len(), count, "labels_for({count}) overallocated");
                assert_eq!(prefix, labels, "labels_up_to({count}) changed label order");
                let len = label_length_for(count, &keys);
                assert!(prefix.iter().all(|label| label.chars().count() == len));
                let unique: std::collections::HashSet<&String> = prefix.iter().collect();
                assert_eq!(
                    unique.len(),
                    prefix.len(),
                    "labels_up_to({count}) had dupes"
                );
            }
        }
    }

    #[test]
    fn label_prefixes_match_full_generation_at_length_boundaries() {
        for keys in [LabelKeys::default(), LabelKeys::from_env("abc")] {
            let radix = keys.len();
            for count in [1, radix, radix + 1, radix.pow(2), radix.pow(2) + 1] {
                let labels = labels_for(count, &keys);
                assert_eq!(
                    labels_up_to(count, &keys),
                    labels,
                    "label order changed at count {count} for radix {radix}"
                );
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
