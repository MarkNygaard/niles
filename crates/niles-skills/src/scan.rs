//! Security scan for skill content.
//!
//! Rejects: null byte, C0/C1 control chars (except \n\r\t), BOM,
//! ZWSP/joiners, RTL override.

use crate::error::{Error, Result};

/// Scan `text` for disallowed characters.
/// Returns `Ok(())` if clean, `Err(Error::ScanFailed)` if not.
pub fn scan(text: &str) -> Result<()> {
    for (idx, ch) in text.chars().enumerate() {
        if let Some(reason) = disallowed_reason(ch, idx) {
            return Err(Error::ScanFailed { reason });
        }
    }
    Ok(())
}

fn disallowed_reason(ch: char, idx: usize) -> Option<String> {
    match ch {
        '\0' => Some(format!("null byte at byte offset {}", idx)),
        '\u{FEFF}' => Some(format!("BOM at byte offset {}", idx)),
        '\u{202E}' => Some(format!("RTL override at byte offset {}", idx)),
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' => {
            Some(format!("zero-width space or joiner at byte offset {}", idx))
        }
        _ => {
            let cp = ch as u32;
            if (0x01..=0x1F).contains(&cp) && !matches!(ch, '\n' | '\r' | '\t') {
                Some(format!("C0 control character at byte offset {}", idx))
            } else if (0x80..=0x9F).contains(&cp) {
                Some(format!("C1 control character at byte offset {}", idx))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_passes() {
        assert!(scan("Hello, world!\nThis is fine.\tReally.").is_ok());
    }

    #[test]
    fn null_byte_rejected() {
        let err = scan("hello\0world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("null byte"), "{msg}");
    }

    #[test]
    fn bom_rejected() {
        let err = scan("\u{FEFF}hello").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("BOM"), "{msg}");
    }

    #[test]
    fn zwsp_rejected() {
        let err = scan("hello\u{200B}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("zero-width"), "{msg}");
    }

    #[test]
    fn zwj_rejected() {
        let err = scan("hello\u{200C}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn zwnj_rejected() {
        let err = scan("hello\u{200D}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn word_joiner_rejected() {
        let err = scan("hello\u{2060}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn rtl_override_rejected() {
        let err = scan("hello\u{202E}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("RTL"), "{msg}");
    }

    #[test]
    fn c0_control_rejected() {
        let err = scan("hello\x01world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("C0"), "{msg}");
    }

    #[test]
    fn c1_control_rejected() {
        let err = scan("hello\u{0080}world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("C1"), "{msg}");
    }

    #[test]
    fn allowed_controls_pass() {
        assert!(scan("line1\nline2\rline3\tcol").is_ok());
    }
}
