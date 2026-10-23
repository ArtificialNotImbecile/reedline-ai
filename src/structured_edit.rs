//! Previewable replacement or insertion, bound to the complete source buffer.
use crate::context::{Context, MAX_CONTEXT_BYTES, unsafe_char};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredSuggestion {
    pub source_buffer: String,
    pub source_cursor: usize,
    pub edit: TextEdit,
}
impl std::fmt::Debug for StructuredSuggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructuredSuggestion")
            .field("source_bytes", &self.source_buffer.len())
            .field("source_cursor", &self.source_cursor)
            .field("start", &self.edit.start)
            .field("end", &self.edit.end)
            .field("replacement_bytes", &self.edit.replacement.len())
            .finish()
    }
}
impl StructuredSuggestion {
    pub fn new(buffer: &str, cursor: usize, edit: TextEdit) -> Result<Self, &'static str> {
        let value = Self {
            source_buffer: buffer.into(),
            source_cursor: cursor,
            edit,
        };
        value.apply(buffer, cursor)?;
        Ok(value)
    }
    /// Call again at acceptance time, not just at request/render time. Full
    /// equality (not a prefix or hash) prevents applying a stale replacement.
    pub fn apply(&self, current: &str, cursor: usize) -> Result<(String, usize), &'static str> {
        Context::new(current, cursor)?;
        if current != self.source_buffer || cursor != self.source_cursor {
            return Err("suggestion is stale: buffer or cursor changed");
        }
        if self.edit.start > self.edit.end
            || !current.is_char_boundary(self.edit.start)
            || !current.is_char_boundary(self.edit.end)
        {
            return Err("edit range must use ordered UTF-8 boundaries within the source buffer");
        }
        let replacement = &self.edit.replacement;
        if replacement.len() > MAX_CONTEXT_BYTES
            || replacement.chars().any(unsafe_char)
            || replacement.contains("```")
        {
            return Err("replacement exceeds limits or contains unsafe formatting");
        }
        let result = format!(
            "{}{}{}",
            &current[..self.edit.start],
            replacement,
            &current[self.edit.end..]
        );
        let cursor = self.edit.start + replacement.len();
        Context::new(&result, cursor)?;
        Ok((result, cursor))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_and_insertion_preserve_suffix() {
        let s = StructuredSuggestion::new(
            "ls 世界",
            2,
            TextEdit {
                start: 0,
                end: 2,
                replacement: "dir".into(),
            },
        )
        .unwrap();
        assert_eq!(s.apply("ls 世界", 2).unwrap(), ("dir 世界".into(), 3));
        assert!(s.apply("ls 宇宙", 2).is_err());
        assert!(s.apply("ls 世界", 3).is_err());
        let s = StructuredSuggestion::new(
            "世.txt",
            3,
            TextEdit {
                start: 3,
                end: 3,
                replacement: "界".into(),
            },
        )
        .unwrap();
        assert_eq!(s.apply("世.txt", 3).unwrap(), ("世界.txt".into(), 6));
    }
    #[test]
    fn serialized_edits_are_revalidated_at_acceptance() {
        for (start, end) in [(1, 3), (3, 1), (0, 99), (usize::MAX, usize::MAX)] {
            assert!(
                StructuredSuggestion::new(
                    "世界",
                    6,
                    TextEdit {
                        start,
                        end,
                        replacement: "a".into()
                    }
                )
                .is_err()
            );
        }
        for replacement in ["a\nb", "\x1b[31m", "x\u{202e}", "```"] {
            assert!(
                StructuredSuggestion::new(
                    "a",
                    1,
                    TextEdit {
                        start: 0,
                        end: 1,
                        replacement: replacement.into()
                    }
                )
                .is_err()
            );
        }
    }
}
