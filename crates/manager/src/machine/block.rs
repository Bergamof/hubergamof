//! A delimited block that HuBergamof keeps in a file of the user (a shell
//! startup file, a Git configuration): it sources or includes a file of its
//! own. Pure text functions; the rest of the file is never modified.

use std::ops::Range;

/// A block start without its end: the file is not ours to rewrite.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UnterminatedBlock;

/// What delimits a block and what it holds.
pub(super) struct Block<'a> {
    /// First line.
    pub start: &'a str,
    /// Last line.
    pub end: &'a str,
    /// What is between the two, each line ending with a newline.
    pub body: &'a str,
}

impl Block<'_> {
    /// The block, ending with a newline.
    pub(super) fn text(&self) -> String {
        format!("{}\n{}{}\n", self.start, self.body, self.end)
    }

    /// Line ranges of every block of `lines`.
    fn ranges(&self, lines: &[&str]) -> Result<Vec<Range<usize>>, UnterminatedBlock> {
        let mut ranges = Vec::new();
        let mut start = None;
        for (index, line) in lines.iter().enumerate() {
            let line = line.trim_end();
            if line == self.start && start.is_none() {
                start = Some(index);
            } else if line == self.end
                && let Some(first) = start.take()
            {
                ranges.push(first..index + 1);
            }
        }
        match start {
            Some(_) => Err(UnterminatedBlock),
            None => Ok(ranges),
        }
    }

    /// `content` with the block: replaced in place when there is one (any
    /// others are dropped), else appended after a blank line.
    ///
    /// # Errors
    /// [`UnterminatedBlock`] when a block has no end.
    pub(super) fn insert(&self, content: &str) -> Result<String, UnterminatedBlock> {
        let lines = lines_of(content);
        let ranges = self.ranges(&lines)?;
        let Some(first) = ranges.first() else {
            let mut appended = content.to_owned();
            if !appended.is_empty() {
                if !appended.ends_with('\n') {
                    appended.push('\n');
                }
                appended.push('\n');
            }
            appended.push_str(&self.text());
            return Ok(appended);
        };
        let mut replaced = String::with_capacity(content.len());
        for (index, line) in lines.iter().enumerate() {
            if index == first.start {
                replaced.push_str(&self.text());
            }
            if !ranges.iter().any(|range| range.contains(&index)) {
                replaced.push_str(line);
            }
        }
        Ok(replaced)
    }

    /// `content` without any block, nor the blank line that separated it
    /// from what precedes.
    ///
    /// # Errors
    /// [`UnterminatedBlock`] when a block has no end.
    pub(super) fn remove(&self, content: &str) -> Result<String, UnterminatedBlock> {
        let lines = lines_of(content);
        let ranges = self.ranges(&lines)?;
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
}

fn lines_of(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT: Block<'static> = Block {
        start: "# >>> hubergamof git >>>",
        end: "# <<< hubergamof git <<<",
        body: "[include]\n\tpath = ~/x\n",
    };

    #[test]
    fn a_block_with_other_markers_is_added_replaced_and_removed_like_any() {
        let original = "[user]\n\tname = Ada\n";
        let with = GIT.insert(original).unwrap();
        assert_eq!(with, format!("{original}\n{}", GIT.text()));
        assert_eq!(GIT.insert(&with).unwrap(), with);
        assert_eq!(GIT.remove(&with).unwrap(), original);
    }

    #[test]
    fn the_markers_of_another_block_are_not_this_one() {
        let other = "# >>> hubergamof aliases >>>\nx\n# <<< hubergamof aliases <<<\n";
        assert_eq!(GIT.remove(other).unwrap(), other);
        assert!(GIT.insert(other).unwrap().ends_with(&GIT.text()));
    }

    #[test]
    fn a_block_without_its_end_is_refused() {
        let broken = "a\n# >>> hubergamof git >>>\n[include]\n";
        assert_eq!(GIT.insert(broken), Err(UnterminatedBlock));
        assert_eq!(GIT.remove(broken), Err(UnterminatedBlock));
    }
}
