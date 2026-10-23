use serde::Serialize;
use std::fmt;

pub const MAX_CONTEXT_BYTES: usize = 8 * 1024;
pub const MAX_INSERTION_BYTES: usize = 1024;

/// A byte-indexed, immutable snapshot. Construction validates the UTF-8 cursor.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Context {
    pub(crate) prefix: String,
    pub(crate) suffix: String,
}

// Do not accidentally include user input in diagnostic output.
impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("prefix_bytes", &self.prefix.len())
            .field("suffix_bytes", &self.suffix.len())
            .finish()
    }
}

impl Context {
    pub fn new(buffer: &str, cursor: usize) -> Result<Self, &'static str> {
        if buffer.len() > MAX_CONTEXT_BYTES {
            return Err("buffer exceeds the 8192-byte privacy limit");
        }
        if !buffer.is_char_boundary(cursor) {
            return Err("cursor must be a valid UTF-8 byte boundary within the buffer");
        }
        if buffer.chars().any(unsafe_char) {
            return Err("only single-line text without terminal controls is supported");
        }
        let (prefix, suffix) = buffer.split_at(cursor);
        Ok(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
        })
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn is_empty(&self) -> bool {
        self.prefix.is_empty() && self.suffix.is_empty()
    }

    /// Insert verbatim at the original cursor without replacing the suffix.
    pub fn insert(&self, insertion: &str) -> Result<String, &'static str> {
        validate_insertion(insertion)?;
        Ok(format!("{}{}{}", self.prefix, insertion, self.suffix))
    }
}

/// Reject rather than strip control characters, multiline output, fences, and
/// bidi/invisible formatting controls. Stripping could silently change commands.
pub fn validate_insertion(text: &str) -> Result<(), &'static str> {
    if text.len() > MAX_INSERTION_BYTES {
        return Err("completion exceeds the 1024-byte output limit");
    }
    if text.chars().any(unsafe_char) || text.contains("```") {
        return Err("completion contains unsafe formatting or terminal controls");
    }
    Ok(())
}

pub fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c,
        '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' |
        '\u{2060}'..='\u{206f}' | '\u{feff}')
}

/// Escape all nonprinting characters before echoing submitted text.
pub fn safe_display(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if unsafe_char(c) {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_cursor_preserves_suffix() {
        let context = Context::new("echo 世界.txt", "echo 世".len()).unwrap();
        assert_eq!(context.prefix, "echo 世");
        assert_eq!(context.suffix, "界.txt");
        assert_eq!(context.insert("你好").unwrap(), "echo 世你好界.txt");
    }

    #[test]
    fn every_utf8_boundary_preserves_mixed_unicode_and_emoji() {
        let buffer = "echo 👩🏽‍💻 café 世界";
        // This example includes a zero-width joiner, which our input safety
        // filter deliberately disallows. Plain emoji/combining text still
        // exercises multibyte boundaries without hidden format characters.
        assert!(Context::new(buffer, 5).is_err());
        let buffer = "echo 🦀 café e\u{301} 世界";
        for cursor in 0..=buffer.len() {
            match Context::new(buffer, cursor) {
                Ok(context) => {
                    assert!(buffer.is_char_boundary(cursor));
                    assert_eq!(
                        context.insert("✓").unwrap(),
                        format!("{}✓{}", &buffer[..cursor], &buffer[cursor..])
                    );
                }
                Err(_) => assert!(!buffer.is_char_boundary(cursor)),
            }
        }
    }

    #[test]
    fn invalid_cursors_and_large_context_are_rejected() {
        assert!(Context::new("世", 1).is_err());
        assert!(Context::new("a", 2).is_err());
        assert!(Context::new("a", usize::MAX).is_err());
        assert!(Context::new(&"a".repeat(MAX_CONTEXT_BYTES + 1), 0).is_err());
    }

    #[test]
    fn unsafe_output_is_rejected_not_rewritten() {
        for text in [
            "a\nb",
            "\x1b[31mred",
            "a\rb",
            "a\t",
            "```sh",
            "x\u{202e}y",
            "x\u{200b}y",
        ] {
            assert!(validate_insertion(text).is_err(), "{text:?}");
        }
        assert!(validate_insertion(&"a".repeat(MAX_INSERTION_BYTES + 1)).is_err());
        assert!(validate_insertion(" --format='世界'").is_ok());
        assert_eq!(safe_display("hi\x1b[31m"), "hi\\u{1b}[31m");
    }

    #[test]
    fn debug_does_not_expose_buffer() {
        assert!(!format!("{:?}", Context::new("PRIVATE", 7).unwrap()).contains("PRIVATE"));
    }
}
