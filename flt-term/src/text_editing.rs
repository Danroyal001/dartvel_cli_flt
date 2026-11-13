//! The text a focused Flutter text field holds, as the platform side sees it.
//!
//! Flutter sends `TextInput.setEditingState` whenever a field's text or
//! selection changes; typed text goes back as `updateEditingState` with the
//! selection replaced. Offsets are UTF-16 code units, as Flutter counts them,
//! so an emoji is two.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditingState {
    pub text: String,
    pub selection_base: i64,
    pub selection_extent: i64,
}

impl EditingState {
    /// Replaces the selection with `inserted` and puts the caret after it.
    /// A selection Flutter marks invalid (-1) inserts at the end.
    pub fn insert(&mut self, inserted: &str) {
        let units: Vec<u16> = self.text.encode_utf16().collect();
        let length = units.len() as i64;
        let (mut start, mut end) = if self.selection_base < 0 || self.selection_extent < 0 {
            (length, length)
        } else {
            (self.selection_base.min(self.selection_extent), self.selection_base.max(self.selection_extent))
        };
        start = start.clamp(0, length);
        end = end.clamp(0, length);
        let mut result: Vec<u16> = units[..start as usize].to_vec();
        result.extend(inserted.encode_utf16());
        let caret = result.len() as i64;
        result.extend_from_slice(&units[end as usize..]);
        self.text = String::from_utf16_lossy(&result);
        self.selection_base = caret;
        self.selection_extent = caret;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str, base: i64, extent: i64) -> EditingState {
        EditingState { text: text.into(), selection_base: base, selection_extent: extent }
    }

    #[test]
    fn typing_at_the_caret() {
        let mut editing = state("helo", 3, 3);
        editing.insert("l");
        assert_eq!(editing, state("hello", 4, 4));
    }

    #[test]
    fn typing_replaces_a_selection_either_way_round() {
        let mut forwards = state("hello world", 6, 11);
        forwards.insert("there");
        assert_eq!(forwards, state("hello there", 11, 11));
        let mut backwards = state("hello world", 11, 6);
        backwards.insert("you");
        assert_eq!(backwards, state("hello you", 9, 9));
    }

    #[test]
    fn offsets_count_utf16_units() {
        // The emoji is two UTF-16 units; the caret after it is at 3.
        let mut editing = state("a😀b", 3, 3);
        editing.insert("é");
        assert_eq!(editing, state("a😀éb", 4, 4));
        editing.insert("😀");
        assert_eq!(editing.selection_base, 6);
        assert_eq!(editing.text, "a😀é😀b");
    }

    #[test]
    fn an_invalid_selection_appends() {
        let mut editing = state("abc", -1, -1);
        editing.insert("d");
        assert_eq!(editing, state("abcd", 4, 4));
    }

    #[test]
    fn a_selection_past_the_end_is_clamped() {
        let mut editing = state("abc", 10, 10);
        editing.insert("d");
        assert_eq!(editing, state("abcd", 4, 4));
    }
}
