//! Security scan for memory content.
//!
//! Guards against common LLM unicode bugs and control-character
//! injection. NOT a prompt-injection heuristic — that is deferred.

use crate::error::{Error, Result};

/// Reject content that contains problematic characters.
///
/// Blocked:
/// - null bytes (`\0`)
/// - C0 controls other than `\n`, `\r`, `\t`
/// - C1 controls
/// - BOM (`\u{FEFF}`)
/// - ZWSP (`\u{200B}`), ZWJ (`\u{200D}`), ZWNBSP (`\u{FEFF}`), WJ (`\u{2060}`)
/// - RTL override (`\u{202E}`)
pub fn scan(content: &str) -> Result<()> {
    let mut byte_offset = 0usize;
    for ch in content.chars() {
        let cp = ch as u32;
        if ch == '\0' {
            return Err(Error::ScanFailed {
                reason: format!("null byte at byte offset {byte_offset}"),
            });
        }
        // C1 controls (U+0080–U+009F)
        if (0x80..=0x9F).contains(&cp) {
            return Err(Error::ScanFailed {
                reason: format!(
                    "C1 control character U+{:04X} at byte offset {byte_offset}",
                    cp
                ),
            });
        }
        // C0 controls except \n \r \t
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            return Err(Error::ScanFailed {
                reason: format!(
                    "control character U+{:04X} at byte offset {byte_offset}",
                    cp
                ),
            });
        }
        // BOM
        if cp == 0xFEFF {
            return Err(Error::ScanFailed {
                reason: format!("BOM at byte offset {byte_offset}"),
            });
        }
        // ZWSP, ZWJ, WJ
        if matches!(cp, 0x200B | 0x200D | 0x2060) {
            return Err(Error::ScanFailed {
                reason: format!(
                    "zero-width character U+{:04X} at byte offset {byte_offset}",
                    cp
                ),
            });
        }
        // RTL override
        if cp == 0x202E {
            return Err(Error::ScanFailed {
                reason: format!("RTL override character at byte offset {byte_offset}"),
            });
        }
        byte_offset += ch.len_utf8();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_passes() {
        assert!(scan("Hello, world!").is_ok());
        assert!(scan("Multi\nline\r\ntext\twith tabs").is_ok());
        assert!(scan("Unicode: æøå 日本語 🎉").is_ok());
    }

    #[test]
    fn null_byte_rejected() {
        let err = scan("hello\0world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("null byte"), "{msg}");
    }

    #[test]
    fn bell_control_rejected() {
        let err = scan("hello\x07world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("control character"), "{msg}");
    }

    #[test]
    fn c1_control_rejected() {
        let err = scan("hello\u{0081}world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("C1 control"), "{msg}");
    }

    #[test]
    fn newline_tab_carriage_return_allowed() {
        assert!(scan("line1\nline2").is_ok());
        assert!(scan("col1\tcol2").is_ok());
        assert!(scan("line1\r\nline2").is_ok());
    }

    #[test]
    fn bom_rejected() {
        let err = scan("\u{FEFF}hello").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("BOM"), "{msg}");
    }

    #[test]
    fn zwsp_rejected() {
        let err = scan("hello\u{200B}world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("zero-width"), "{msg}");
    }

    #[test]
    fn zwj_rejected() {
        let err = scan("hello\u{200D}world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("zero-width"), "{msg}");
    }

    #[test]
    fn wj_rejected() {
        let err = scan("hello\u{2060}world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("zero-width"), "{msg}");
    }

    #[test]
    fn rtl_override_rejected() {
        let err = scan("hello\u{202E}world").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("RTL override"), "{msg}");
    }

    #[test]
    fn empty_string_passes() {
        assert!(scan("").is_ok());
    }
}
