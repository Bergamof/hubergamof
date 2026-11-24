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

    /// `content` with the block as its first lines, followed by a blank line:
    /// where the file reads it before everything else (an `Include` of
    /// `ssh_config` must come before any `Host`, and its hosts win by coming
    /// first). Any block already in the file is removed first, so that the
    /// block is never twice nor left where the person moved it.
    ///
    /// # Errors
    /// [`UnterminatedBlock`] when a block has no end.
    pub(super) fn place_first(&self, content: &str) -> Result<String, UnterminatedBlock> {
        let rest = self.remove(content)?;
        if rest.is_empty() {
            return Ok(self.text());
        }
        Ok(format!("{}\n{rest}", self.text()))
    }

    /// `content` without any block, nor the blank line that separated it
    /// from what precedes, or follows it when it is the first thing of the
    /// file.
    ///
    /// # Errors
    /// [`UnterminatedBlock`] when a block has no end.
    pub(super) fn remove(&self, content: &str) -> Result<String, UnterminatedBlock> {
        let lines = lines_of(content);
        let ranges = self.ranges(&lines)?;
        let mut kept = String::with_capacity(content.len());
        for (index, line) in lines.iter().enumerate() {
            let in_block = ranges.iter().any(|range| range.contains(&index));
            let before_block = ranges.iter().any(|range| range.start == index + 1);
            let after_first_block = ranges
                .iter()
                .any(|range| range.start == 0 && range.end == index);
            let separator = line.trim().is_empty() && (before_block || after_first_block);
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
    fn removing_a_block_takes_the_blank_line_before_it_or_after_it_when_it_is_first_and_no_other() {
        let block = GIT.text();
        let remove = |content: String| GIT.remove(&content).unwrap();
        // In the middle: the blank line before goes, the one after stays.
        assert_eq!(remove(format!("a\n\n{block}\nb\n")), "a\n\nb\n");
        assert_eq!(remove(format!("a\n{block}b\n")), "a\nb\n");
        // First: the blank line after goes, and only one.
        assert_eq!(remove(format!("{block}\nrest")), "rest");
        assert_eq!(remove(format!("{block}\n\nrest")), "\nrest");
        assert_eq!(remove(format!("{block}rest")), "rest");
        // Alone.
        assert_eq!(remove(block.clone()), "");
        // The blank line after a block that is not first is the person's.
        assert_eq!(remove(format!("a\n{block}\nb\n")), "a\n\nb\n");
    }

    #[test]
    fn a_block_placed_first_comes_before_everything_and_goes_back_to_the_original() {
        for original in [
            "Host a\n\tUser u\n",
            "",
            "# c\n\nHost a\n",
            "\nHost a\n",
            "Host a",
            "\n",
            "Host a\r\n\tUser u\r\n",
        ] {
            let placed = GIT.place_first(original).unwrap();
            assert!(placed.starts_with(&GIT.text()), "{original:?}");
            assert_eq!(
                GIT.place_first(&placed).unwrap(),
                placed,
                "twice: {original:?}"
            );
            assert_eq!(GIT.remove(&placed).unwrap(), original, "{original:?}");
        }
    }

    #[test]
    fn a_block_without_its_end_is_refused_when_placed_first() {
        let broken = format!("{}\nInclude x\n", GIT.start);
        assert_eq!(GIT.place_first(&broken), Err(UnterminatedBlock));
    }

    #[test]
    fn a_block_moved_by_the_person_is_placed_first_again() {
        let moved = format!("Host a\n{}Host b\n", GIT.text());
        let placed = GIT.place_first(&moved).unwrap();
        assert!(placed.starts_with(&GIT.text()));
        assert_eq!(placed.matches(GIT.start).count(), 1);
        assert!(placed.ends_with("Host a\nHost b\n"));
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
