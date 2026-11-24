//! What the validations of free text share: which characters show nothing and
//! what a blank text is. The interface checks the same (`alias-validation.ts`).

/// Zero width no-break space: not white space for Rust, but for JavaScript,
/// which the interface runs on; both must agree on what a blank command is.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// Characters that show nothing or reorder the text around them: a command
/// containing one reads as something other than what it runs (the same set as
/// the interface, `alias-validation.ts`).
pub(crate) fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | BYTE_ORDER_MARK)
}

pub(crate) fn is_blank(command: &str) -> bool {
    command
        .chars()
        .all(|c| c.is_whitespace() || c == BYTE_ORDER_MARK)
}
