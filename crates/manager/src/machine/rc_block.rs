//! The block HuBergamof keeps in the shell startup files (`.bashrc`,
//! `.zshrc`): it sources the managed aliases file. Pure text functions; the
//! rest of the file is never modified.

use std::ops::Range;

use super::shell::MANAGED_FILE;

/// First line of the block.
pub(super) const BLOCK_START: &str = "# >>> hubergamof aliases >>>";

/// Last line of the block.
pub(super) const BLOCK_END: &str = "# <<< hubergamof aliases <<<";

/// A block start without its end: the file is not ours to rewrite.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UnterminatedBlock;

/// The block, ending with a newline. The managed file is sourced only when it
/// exists, so that a removed file never makes the shell complain.
fn block() -> String {
    format!(
        "{BLOCK_START}\nif [ -f \"$HOME/{MANAGED_FILE}\" ]; then . \"$HOME/{MANAGED_FILE}\"; fi\n{BLOCK_END}\n"
    )
}

/// Line ranges of every block of `lines`.
fn block_ranges(lines: &[&str]) -> Result<Vec<Range<usize>>, UnterminatedBlock> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, line) in lines.iter().enumerate() {
        match (line.trim_end(), start) {
            (BLOCK_START, None) => start = Some(index),
            (BLOCK_END, Some(first)) => {
                ranges.push(first..index + 1);
                start = None;
            }
            _ => {}
        }
    }
    match start {
        Some(_) => Err(UnterminatedBlock),
        None => Ok(ranges),
    }
}

fn lines_of(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

/// `content` with the block: replaced in place when there is one (any others
/// are dropped), else appended after a blank line.
///
/// # Errors
/// [`UnterminatedBlock`] when a block has no end.
pub(super) fn with_block(content: &str) -> Result<String, UnterminatedBlock> {
    let lines = lines_of(content);
    let ranges = block_ranges(&lines)?;
    let Some(first) = ranges.first() else {
        let mut appended = content.to_owned();
        if !appended.is_empty() {
            if !appended.ends_with('\n') {
                appended.push('\n');
            }
            appended.push('\n');
        }
        appended.push_str(&block());
        return Ok(appended);
    };
    let mut replaced = String::with_capacity(content.len());
    for (index, line) in lines.iter().enumerate() {
        if index == first.start {
            replaced.push_str(&block());
        }
        if !ranges.iter().any(|range| range.contains(&index)) {
            replaced.push_str(line);
        }
    }
    Ok(replaced)
}

/// `content` without any block, nor the blank line that separated it from
/// what precedes.
///
/// # Errors
/// [`UnterminatedBlock`] when a block has no end.
pub(super) fn without_block(content: &str) -> Result<String, UnterminatedBlock> {
    let lines = lines_of(content);
    let ranges = block_ranges(&lines)?;
    let mut kept = String::with_capacity(content.len());
    for (index, line) in lines.iter().enumerate() {
        let in_block = ranges.iter().any(|range| range.contains(&index));
        let separator =
            line.trim().is_empty() && ranges.iter().any(|range| range.start == index + 1);
        if !in_block && !separator {
            kept.push_str(line);
        }
    }
    Ok(kept)
}

/// Lines of `content` outside every block, for reading what the user wrote.
pub(super) fn outside_blocks(content: &str) -> impl Iterator<Item = &str> {
    let mut inside = false;
    content.lines().filter(move |line| {
        match (line.trim_end(), inside) {
            (BLOCK_START, _) => inside = true,
            (BLOCK_END, true) => {
                inside = false;
                return false;
            }
            _ => {}
        }
        !inside
    })
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

    #[test]
    fn lines_outside_the_block_are_listed() {
        let content = format!("a\n{}b\n", block());
        assert_eq!(outside_blocks(&content).collect::<Vec<_>>(), ["a", "b"]);
    }
}
