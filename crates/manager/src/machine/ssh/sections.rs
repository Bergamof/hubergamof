//! Reading `~/.ssh/config` as the sections `ssh` sees: what comes before the
//! first `Host` or `Match`, then one section per `Host` or `Match`, up to the
//! next one. Pure text functions, no program is run.

use std::ops::Range;

/// What starts a line of `~/.ssh/config` that HuBergamof has commented out
/// because it took the host over. It is a comment for `ssh`, and the marker
/// tells it from a comment of the user, so that it can be given back.
pub(super) const CLAIMED_MARKER: &str = "#hubergamof-off# ";

/// How a section starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Header {
    /// What comes before the first `Host` or `Match`.
    Preamble,
    /// `Host` and its patterns.
    Host(Vec<String>),
    /// `Match`: never taken over, and it ends the section before it.
    Match,
}

/// A section: its header and the lines it covers (the header line included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Section {
    pub header: Header,
    /// Whether the header line is one HuBergamof commented out.
    pub marked: bool,
    pub lines: Range<usize>,
}

/// The keyword (lowercase) and the value of an option line: `Key value` or
/// `Key=value`. Nothing for a blank or comment line.
pub(super) fn option_of(line: &str) -> Option<(String, String)> {
    let text = line.trim();
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let end = text
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(text.len());
    let value = text[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    Some((text[..end].to_ascii_lowercase(), unquoted(value.trim_end())))
}

/// `value` without the double quotes that surround it.
fn unquoted(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

/// The header a line starts, and whether HuBergamof commented it out.
fn header_of(line: &str) -> Option<(Header, bool)> {
    let (text, marked) = match line.strip_prefix(CLAIMED_MARKER) {
        Some(inner) => (inner, true),
        None => (line, false),
    };
    let (keyword, value) = option_of(text)?;
    match keyword.as_str() {
        "host" => Some((
            Header::Host(value.split_whitespace().map(str::to_owned).collect()),
            marked,
        )),
        "match" => Some((Header::Match, marked)),
        _ => None,
    }
}

/// The lines of `content`, line ends kept.
pub(super) fn lines_of(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

/// The sections of `lines`, in order, covering all of them.
pub(super) fn sections(lines: &[&str]) -> Vec<Section> {
    let mut found = vec![Section {
        header: Header::Preamble,
        marked: false,
        lines: 0..0,
    }];
    for (index, line) in lines.iter().enumerate() {
        let text = line.trim_end_matches(['\n', '\r']);
        if let Some((header, marked)) = header_of(text) {
            if let Some(last) = found.last_mut() {
                last.lines.end = index;
            }
            found.push(Section {
                header,
                marked,
                lines: index..index,
            });
        }
    }
    if let Some(last) = found.last_mut() {
        last.lines.end = lines.len();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(content: &str) -> Vec<(Header, bool, Range<usize>)> {
        sections(&lines_of(content))
            .into_iter()
            .map(|section| (section.header, section.marked, section.lines))
            .collect()
    }

    #[test]
    fn an_option_is_split_at_a_space_or_an_equal_sign_and_unquoted() {
        for line in [
            "HostName example.org",
            "  hostname\texample.org  ",
            "HOSTNAME=example.org",
            "HostName = \"example.org\"",
        ] {
            assert_eq!(
                option_of(line),
                Some(("hostname".into(), "example.org".into())),
                "{line:?}"
            );
        }
        assert_eq!(option_of("# a comment"), None);
        assert_eq!(option_of("   \n"), None);
    }

    #[test]
    fn the_file_is_cut_at_each_host_and_match() {
        let content = "Include x\n\nHost a\n\tUser u\nHost b c\nMatch all\n\tUser m\n";
        assert_eq!(
            headers(content),
            [
                (Header::Preamble, false, 0..2),
                (Header::Host(vec!["a".into()]), false, 2..4),
                (Header::Host(vec!["b".into(), "c".into()]), false, 4..5),
                (Header::Match, false, 5..7),
            ]
        );
    }

    #[test]
    fn a_header_commented_out_by_hubergamof_is_still_a_header_and_marked() {
        let content = "#hubergamof-off# Host a\n#hubergamof-off# \tUser u\n# Host b\n";
        assert_eq!(
            headers(content),
            [
                (Header::Preamble, false, 0..0),
                (Header::Host(vec!["a".into()]), true, 0..3),
            ]
        );
    }

    #[test]
    fn a_file_without_header_is_one_preamble() {
        assert_eq!(
            headers("Compression yes\n"),
            [(Header::Preamble, false, 0..1)]
        );
        assert_eq!(headers(""), [(Header::Preamble, false, 0..0)]);
    }
}
