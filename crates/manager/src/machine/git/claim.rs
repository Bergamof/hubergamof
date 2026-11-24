//! Commenting out the definitions of aliases that HuBergamof took over from
//! the Git configuration of the user, so that switching one off really
//! switches it off: the files of HuBergamof are read last and win, but an
//! alias that is off must not come back from the line it was taken from.
//!
//! The rule has no memory: a line commented out by HuBergamof carries
//! [`CLAIMED_MARKER`], which says what to give back once the alias is no
//! longer in the configuration. Pure text functions.

use std::collections::HashSet;

/// What starts a line of the Git configuration of the user that HuBergamof
/// has commented out. It is a comment for Git, and the marker tells it from a
/// comment of the user, so that it can be given back.
pub(super) const CLAIMED_MARKER: &str = "#hubergamof-off# ";

/// The name of the section that holds the aliases.
const ALIAS_SECTION: &str = "alias";

const LINE_ENDS: [char; 2] = ['\n', '\r'];

/// The lowercase name of the alias defined by `line` (`co = checkout`), if
/// `line` is a plain definition.
fn defined_alias(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with(['#', ';', '[']) {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim_end();
    (!key.is_empty() && !key.contains(char::is_whitespace)).then(|| key.to_ascii_lowercase())
}

/// Whether `line` opens the `[alias]` section (a section with a sub-section,
/// `[alias "x"]`, is another one), and whether it opens a section at all.
fn section_header(line: &str) -> Option<bool> {
    let trimmed = line.trim_start();
    let inside = trimmed.strip_prefix('[')?.split_once(']')?.0;
    Some(inside.trim().eq_ignore_ascii_case(ALIAS_SECTION))
}

/// `content` with the definitions of the aliases of `names` (lowercase)
/// commented out, and the lines commented out earlier whose alias is no longer
/// in `names` given back.
///
/// Only a plain definition on a line of its own in an `[alias]` section is
/// touched. One whose value goes on over the next lines (a trailing `\`) is
/// left as it is, and so is a definition on the line of the section header.
pub(super) fn claim_originals(content: &str, names: &HashSet<String>) -> String {
    let mut claimed = String::with_capacity(content.len());
    let mut in_alias_section = false;
    let mut continued = false;
    for line in content.split_inclusive('\n') {
        let text = line.trim_end_matches(LINE_ENDS);
        let mut rewritten = None;
        if continued {
            // The rest of a value: not a definition of its own.
        } else if let Some(original) = text.strip_prefix(CLAIMED_MARKER) {
            if in_alias_section
                && defined_alias(original).is_some_and(|name| !names.contains(&name))
            {
                rewritten = Some(line[CLAIMED_MARKER.len()..].to_owned());
            }
        } else if let Some(is_alias) = section_header(text) {
            in_alias_section = is_alias;
        } else if in_alias_section
            && !text.ends_with('\\')
            && defined_alias(text).is_some_and(|name| names.contains(&name))
        {
            rewritten = Some(format!("{CLAIMED_MARKER}{line}"));
        }
        // A comment does not continue on the next line, a value does.
        continued = !text.trim_start().starts_with(['#', ';']) && text.ends_with('\\');
        claimed.push_str(rewritten.as_deref().unwrap_or(line));
    }
    claimed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> HashSet<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_claimed_definition_is_commented_out_and_the_rest_is_left_alone() {
        let content = "[user]\n\tname = Ada\n[alias]\n\tco = checkout\n\tst = status\n";
        assert_eq!(
            claim_originals(content, &names(&["co"])),
            "[user]\n\tname = Ada\n[alias]\n#hubergamof-off# \tco = checkout\n\tst = status\n"
        );
    }

    #[test]
    fn claiming_twice_changes_nothing_more() {
        let content = "[alias]\n\tco = checkout\n";
        let once = claim_originals(content, &names(&["co"]));
        assert_eq!(claim_originals(&once, &names(&["co"])), once);
    }

    #[test]
    fn a_definition_is_given_back_when_its_alias_leaves_the_configuration() {
        let content = "[alias]\n\tco = checkout\n";
        let claimed = claim_originals(content, &names(&["co"]));
        assert_eq!(claim_originals(&claimed, &names(&[])), content);
        assert_eq!(claim_originals(&claimed, &names(&["st"])), content);
    }

    #[test]
    fn names_are_compared_without_regard_to_case() {
        let content = "[ALIAS]\n\tCo = checkout\n";
        let claimed = claim_originals(content, &names(&["co"]));
        assert_eq!(claimed, "[ALIAS]\n#hubergamof-off# \tCo = checkout\n");
        assert_eq!(claim_originals(&claimed, &names(&[])), content);
    }

    #[test]
    fn only_the_alias_section_is_looked_at() {
        let content = "[core]\n\tco = x\n[alias \"sub\"]\n\tco = y\n[alias]\n\tother = z\n";
        assert_eq!(claim_originals(content, &names(&["co"])), content);
    }

    #[test]
    fn several_alias_sections_are_all_looked_at() {
        let content = "[alias]\n\ta = b\n[core]\n\tx = y\n[alias]\n\tco = checkout\n";
        assert_eq!(
            claim_originals(content, &names(&["co", "a"])),
            "[alias]\n#hubergamof-off# \ta = b\n[core]\n\tx = y\n[alias]\n#hubergamof-off# \tco = checkout\n"
        );
    }

    #[test]
    fn comments_and_lines_without_a_value_are_not_definitions() {
        let content = "[alias]\n\t# co = checkout\n\t; co = checkout\n\tco\n";
        assert_eq!(claim_originals(content, &names(&["co"])), content);
    }

    #[test]
    fn a_value_that_goes_on_over_the_next_lines_is_left_alone_with_its_rest() {
        let content = "[alias]\n\tco = checkout \\\n\t  --quiet\n\tst = status\n";
        assert_eq!(
            claim_originals(content, &names(&["co", "st"])),
            "[alias]\n\tco = checkout \\\n\t  --quiet\n#hubergamof-off# \tst = status\n"
        );
    }

    #[test]
    fn the_rest_of_a_value_is_never_taken_for_a_definition() {
        let content = "[alias]\n\tlg = log \\\n\tco = fake\n";
        assert_eq!(claim_originals(content, &names(&["co"])), content);
    }

    #[test]
    fn a_definition_on_the_line_of_the_section_header_is_left_alone() {
        let content = "[alias] co = checkout\n";
        assert_eq!(claim_originals(content, &names(&["co"])), content);
    }

    #[test]
    fn line_endings_are_kept_and_the_last_line_needs_no_newline() {
        let content = "[alias]\r\n\tco = checkout\r\n\tst = status";
        assert_eq!(
            claim_originals(content, &names(&["co", "st"])),
            "[alias]\r\n#hubergamof-off# \tco = checkout\r\n#hubergamof-off# \tst = status"
        );
    }

    #[test]
    fn the_include_block_of_hubergamof_is_not_touched() {
        let content = "# >>> hubergamof git >>>\n[include]\n\tpath = ~/.config/hubergamof/gitconfig\n# <<< hubergamof git <<<\n";
        assert_eq!(claim_originals(content, &names(&["path"])), content);
    }

    #[test]
    fn a_marked_line_outside_an_alias_section_is_not_given_back() {
        let content = "[core]\n#hubergamof-off# \tco = checkout\n";
        assert_eq!(claim_originals(content, &names(&[])), content);
    }
}
