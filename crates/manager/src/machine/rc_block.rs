//! The block HuBergamof keeps in the shell startup files (`.bashrc`,
//! `.zshrc`): it sources the managed aliases file.

use super::block::{Block, UnterminatedBlock};
use super::shell::MANAGED_FILE;

/// First line of the block.
pub(super) const BLOCK_START: &str = "# >>> hubergamof aliases >>>";

/// Last line of the block.
pub(super) const BLOCK_END: &str = "# <<< hubergamof aliases <<<";

/// What the block holds. The managed file is sourced only when it exists, so
/// that a removed file never makes the shell complain.
fn body() -> String {
    format!("if [ -f \"$HOME/{MANAGED_FILE}\" ]; then . \"$HOME/{MANAGED_FILE}\"; fi\n")
}

fn spec(body: &str) -> Block<'_> {
    Block {
        start: BLOCK_START,
        end: BLOCK_END,
        body,
    }
}

/// The block, ending with a newline.
#[cfg(test)]
fn block() -> String {
    spec(&body()).text()
}

/// `content` with the block, see [`Block::insert`].
///
/// # Errors
/// [`UnterminatedBlock`] when a block has no end.
pub(super) fn with_block(content: &str) -> Result<String, UnterminatedBlock> {
    spec(&body()).insert(content)
}

/// `content` without any block, see [`Block::remove`].
///
/// # Errors
/// [`UnterminatedBlock`] when a block has no end.
pub(super) fn without_block(content: &str) -> Result<String, UnterminatedBlock> {
    spec(&body()).remove(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_appended_after_a_blank_line() {
        let with = with_block("export A=1\n").unwrap();
        assert_eq!(with, format!("export A=1\n\n{}", block()));
    }

    #[test]
    fn the_block_is_appended_to_a_file_without_final_newline_and_to_an_empty_one() {
        assert_eq!(
            with_block("export A=1").unwrap(),
            format!("export A=1\n\n{}", block())
        );
        assert_eq!(with_block("").unwrap(), block());
    }

    #[test]
    fn adding_twice_changes_nothing_more() {
        let once = with_block("a\nb\n").unwrap();
        assert_eq!(with_block(&once).unwrap(), once);
    }

    #[test]
    fn removing_restores_the_original_file() {
        for original in ["export A=1\n", "a\n\nb\n", "a\n\n", ""] {
            let with = with_block(original).unwrap();
            assert_eq!(without_block(&with).unwrap(), original, "{original:?}");
        }
    }

    #[test]
    fn what_surrounds_the_block_is_untouched() {
        let content = format!("before\n{}after\n", block());
        assert_eq!(without_block(&content).unwrap(), "before\nafter\n");
        assert_eq!(with_block(&content).unwrap(), content);
    }

    #[test]
    fn an_outdated_block_is_replaced_in_place() {
        let content = format!("a\n{BLOCK_START}\nold\n{BLOCK_END}\nb\n");
        assert_eq!(with_block(&content).unwrap(), format!("a\n{}b\n", block()));
    }

    #[test]
    fn a_file_without_block_is_left_as_it_is_when_removing() {
        assert_eq!(without_block("a\n\nb").unwrap(), "a\n\nb");
    }

    #[test]
    fn an_unterminated_block_is_refused() {
        let content = format!("a\n{BLOCK_START}\nb\n");
        assert_eq!(with_block(&content), Err(UnterminatedBlock));
        assert_eq!(without_block(&content), Err(UnterminatedBlock));
    }

    #[test]
    fn windows_line_endings_are_recognised() {
        let content = format!("a\r\n{BLOCK_START}\r\nx\r\n{BLOCK_END}\r\n");
        assert_eq!(without_block(&content).unwrap(), "a\r\n");
    }
}
