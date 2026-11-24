//! Commenting out the `Host` sections that HuBergamof took over from
//! `~/.ssh/config`, so that switching a host off really switches it off: the
//! file of HuBergamof comes first and wins, but a host that is off must not
//! come back from the section it was taken from.
//!
//! The rule has no memory: a line commented out by HuBergamof carries
//! [`CLAIMED_MARKER`], which says what to give back once the host is no longer
//! in the configuration. Pure text functions.

use std::collections::{HashMap, HashSet};

use hubergamof_protocol::AdvancedSetting;

use super::sections::{CLAIMED_MARKER, Header, lines_of, sections, split_options};

/// The alias of a section that is one host: a single name, no joker.
fn single_alias(patterns: &[String]) -> Option<&str> {
    match patterns {
        [alias] if !alias.contains(['*', '?', '!']) => Some(alias),
        _ => None,
    }
}

/// Whether `lines` hold nothing but what HuBergamof can carry and give back as
/// it was written (see [`split_options`]): the options of its fields and the
/// options written by the person, with their quotes in pairs. `marked`: the
/// lines are commented out by HuBergamof.
///
/// No option is refused for what it does: the one thing that stops a section
/// is what cannot be carried without loss. What that lets a marker copied from
/// another computer do is in `docs/security.md`.
fn only_carried_options(lines: &[&str], marked: bool) -> bool {
    let inner = |line: &&str| -> String {
        let text = line.trim_end_matches(['\n', '\r']);
        // Only a section HuBergamof commented out has its marker taken off: in another one a
        // line with the marker is a comment, as `ssh` reads it.
        let text = if marked {
            text.strip_prefix(CLAIMED_MARKER).unwrap_or(text)
        } else {
            text
        };
        text.to_owned()
    };
    let texts: Vec<String> = lines.iter().map(inner).collect();
    texts.iter().all(|text| text.matches('"').count() % 2 == 0)
        && split_options(texts.iter().map(String::as_str))
            .lost
            .is_empty()
}

/// Whether every non-blank line of `lines` is commented out by HuBergamof.
fn all_commented_out(lines: &[&str]) -> bool {
    lines
        .iter()
        .all(|line| line.trim().is_empty() || line.starts_with(CLAIMED_MARKER))
}

/// Whether every option of `carried` is also among `written`, by name (without
/// regard to case) and value: the host of the configuration says all that the
/// section it comes from says.
fn is_written_by(carried: &[AdvancedSetting], written: &[AdvancedSetting]) -> bool {
    carried.iter().all(|option| {
        written
            .iter()
            .any(|other| other.key.eq_ignore_ascii_case(&option.key) && other.value == option.value)
    })
}

/// The names of `hosts` (alias → options) that name a host of `content` which
/// HuBergamof cannot give its place to: its section holds options it cannot
/// carry, or options the host does not have (taking the section over would
/// lose them, and leaving it would merge it with the new host).
pub(super) fn untakeable_names<'a>(
    content: &str,
    hosts: &HashMap<&'a str, &[AdvancedSetting]>,
) -> Vec<&'a str> {
    let lines = lines_of(content);
    let mut found = Vec::new();
    for section in sections(&lines) {
        let Header::Host(patterns) = &section.header else {
            continue;
        };
        let named = single_alias(patterns).and_then(|alias| hosts.get_key_value(alias));
        let Some((alias, written)) = named else {
            continue;
        };
        let own = &lines[section.lines.clone()];
        let carried = split_options(own.iter().copied());
        if !section.marked
            && (!only_carried_options(own, false) || !is_written_by(&carried.free, written))
            && !found.contains(alias)
        {
            found.push(*alias);
        }
    }
    found
}

/// `content` with the `Host` sections of the hosts of `aliases` commented out,
/// and the sections commented out earlier whose host is no longer in
/// `aliases` given back.
///
/// Only a section that is one host (a single name, no joker) with nothing but
/// the options HuBergamof manages is touched. A
/// `Match` section is never taken, and what comes before the first `Host`
/// (the include of HuBergamof among it) is left as it is.
pub(super) fn claim_originals(content: &str, aliases: &HashSet<&str>) -> String {
    let lines = lines_of(content);
    let mut claimed = String::with_capacity(content.len());
    for section in sections(&lines) {
        let own = &lines[section.lines.clone()];
        let Header::Host(patterns) = &section.header else {
            claimed.extend(own.iter().copied());
            continue;
        };
        let alias = single_alias(patterns);
        let named = alias.is_some_and(|alias| aliases.contains(alias));
        if !section.marked && named && only_carried_options(own, false) {
            for line in own {
                if line.trim().is_empty() {
                    claimed.push_str(line);
                } else {
                    claimed.push_str(CLAIMED_MARKER);
                    claimed.push_str(line);
                }
            }
        } else if section.marked
            && alias.is_some()
            && !named
            && all_commented_out(own)
            && only_carried_options(own, true)
        {
            for line in own {
                claimed.push_str(line.strip_prefix(CLAIMED_MARKER).unwrap_or(line));
            }
        } else {
            claimed.extend(own.iter().copied());
        }
    }
    claimed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(list: &[&'a str]) -> HashSet<&'a str> {
        list.iter().copied().collect()
    }

    /// Hosts of the configuration that have no option of their own.
    fn bare<'a>(list: &[&'a str]) -> HashMap<&'a str, &'static [AdvancedSetting]> {
        list.iter().map(|name| (*name, &[][..])).collect()
    }

    const FILE: &str = "Include other\n\nHost prod\n\tHostName 10.0.0.1\n\tUser deploy\n\nHost *\n\tCompression yes\n";

    #[test]
    fn a_claimed_host_is_commented_out_whole_and_the_rest_is_left_alone() {
        assert_eq!(
            claim_originals(FILE, &names(&["prod"])),
            "Include other\n\n#hubergamof-off# Host prod\n#hubergamof-off# \tHostName 10.0.0.1\n\
             #hubergamof-off# \tUser deploy\n\nHost *\n\tCompression yes\n"
        );
    }

    #[test]
    fn claiming_twice_changes_nothing_more() {
        let once = claim_originals(FILE, &names(&["prod"]));
        assert_eq!(claim_originals(&once, &names(&["prod"])), once);
    }

    #[test]
    fn a_host_is_given_back_when_it_leaves_the_configuration() {
        let claimed = claim_originals(FILE, &names(&["prod"]));
        assert_eq!(claim_originals(&claimed, &names(&[])), FILE);
        assert_eq!(claim_originals(&claimed, &names(&["other"])), FILE);
    }

    #[test]
    fn a_host_still_in_the_configuration_stays_commented_out() {
        let claimed = claim_originals(FILE, &names(&["prod"]));
        assert_eq!(claim_originals(&claimed, &names(&["prod"])), claimed);
    }

    #[test]
    fn a_section_with_several_names_or_a_joker_is_never_touched() {
        let content = "Host a b\n\tUser u\nHost web-*\n\tUser w\nHost !x\n\tUser x\n";
        assert_eq!(
            claim_originals(content, &names(&["a", "b", "web-*", "!x", "a b"])),
            content
        );
    }

    #[test]
    fn a_match_section_ends_the_host_before_it_and_is_left_alone() {
        let content = "Host a\n\tUser u\nMatch all\n\tUser m\n";
        assert_eq!(
            claim_originals(content, &names(&["a"])),
            "#hubergamof-off# Host a\n#hubergamof-off# \tUser u\nMatch all\n\tUser m\n"
        );
    }

    #[test]
    fn names_are_compared_exactly() {
        let content = "Host Prod\n\tUser u\n";
        assert_eq!(claim_originals(content, &names(&["prod"])), content);
    }

    #[test]
    fn line_endings_are_kept_and_the_last_line_needs_no_newline() {
        let content = "Host a\r\n\tUser u";
        let claimed = claim_originals(content, &names(&["a"]));
        assert_eq!(
            claimed,
            "#hubergamof-off# Host a\r\n#hubergamof-off# \tUser u"
        );
        assert_eq!(claim_originals(&claimed, &names(&[])), content);
    }

    #[test]
    fn a_section_with_options_written_by_the_person_is_taken_and_given_back_as_it_was() {
        for option in [
            "ProxyCommand nc %h %p",
            "ProxyCommand=nc %h %p",
            "LocalCommand curl x | sh",
            "XAuthLocation /tmp/x",
            "Include other",
            "StrictHostKeyChecking no",
            "SendEnv \"A B\"",
            "Compression yes",
        ] {
            let content = format!("Host prod\n\tUser u\n\t{option}\n");
            let claimed = claim_originals(&content, &names(&["prod"]));
            assert!(
                claimed.starts_with("#hubergamof-off# Host prod\n"),
                "{option}"
            );
            // The host of the configuration says what the section says: it is taken as it is.
            let options = split_options(content.lines()).free;
            assert_eq!(
                untakeable_names(&content, &HashMap::from([("prod", options.as_slice())])),
                Vec::<&str>::new(),
                "{option}"
            );
            // One that does not would lose them: refused.
            assert_eq!(
                untakeable_names(&content, &bare(&["prod"])),
                ["prod"],
                "{option}"
            );
            assert_eq!(claim_originals(&claimed, &names(&[])), content, "{option}");
        }
    }

    #[test]
    fn a_section_with_what_cannot_be_carried_is_never_taken_nor_given_back() {
        for option in [
            "Compression yes\n\tCompression no",
            "IdentityFile k1\n\tIdentityFile k2",
            "ForwardAgent /tmp/agent.sock",
            "Port ssh",
            "SendEnv \"A",
            "Bad@Key yes",
            "\"ProxyCommand\" nc %h %p",
            "Pro\"xy\"Command nc",
            "'ProxyCommand' nc",
            "Compression =",
        ] {
            let content = format!("Host prod\n\tUser u\n\t{option}\n");
            assert_eq!(
                claim_originals(&content, &names(&["prod"])),
                content,
                "{option}"
            );
            assert_eq!(
                untakeable_names(&content, &bare(&["prod", "other"])),
                ["prod"]
            );
            let mut marked = String::new();
            for line in content.lines() {
                marked.push_str(CLAIMED_MARKER);
                marked.push_str(line);
                marked.push('\n');
            }
            assert_eq!(claim_originals(&marked, &names(&[])), marked, "{option}");
        }
    }

    #[test]
    fn the_names_that_cannot_be_taken_are_those_of_the_configuration_only() {
        let content = "Host a\n\tIdentityFile x\n\tIdentityFile y\nHost b\n\tUser u\nHost c d\n\tIdentityFile x\n\tIdentityFile y\n";
        assert_eq!(untakeable_names(content, &bare(&["a", "b", "c"])), ["a"]);
        assert_eq!(untakeable_names(content, &bare(&[])), Vec::<&str>::new());
        // What HuBergamof commented out is not a conflict: it is its own.
        let claimed = "#hubergamof-off# Host a\n#hubergamof-off# \tUser u\n";
        assert_eq!(untakeable_names(claimed, &bare(&["a"])), Vec::<&str>::new());
    }

    #[test]
    fn a_host_with_a_comment_after_its_name_or_quoted_keywords_is_still_taken() {
        let content = "Host prod # the production one\n\tUser u\n\"Host\" \"web\"\n\tUser w\n";
        let claimed = claim_originals(content, &names(&["prod", "web"]));
        assert_eq!(
            claimed,
            "#hubergamof-off# Host prod # the production one\n#hubergamof-off# \tUser u\n\
             #hubergamof-off# \"Host\" \"web\"\n#hubergamof-off# \tUser w\n"
        );
        assert_eq!(claim_originals(&claimed, &names(&[])), content);
    }

    #[test]
    fn a_section_that_was_edited_or_could_break_the_file_is_not_given_back() {
        for content in [
            "#hubergamof-off# Host x\n\tUser u\n",
            "#hubergamof-off# Host x\n#hubergamof-off# \tUser \"u\n",
            "#hubergamof-off# Host x y\n#hubergamof-off# \tUser u\n",
        ] {
            assert_eq!(
                claim_originals(content, &names(&[])),
                content,
                "{content:?}"
            );
        }
    }

    #[test]
    fn a_host_without_a_body_is_taken_and_given_back_with_or_without_a_last_newline() {
        for content in ["Host a\n", "Host a", "Host a\nHost b\n"] {
            let claimed = claim_originals(content, &names(&["a"]));
            assert!(
                claimed.starts_with("#hubergamof-off# Host a"),
                "{content:?}"
            );
            assert_eq!(
                claim_originals(&claimed, &names(&[])),
                content,
                "{content:?}"
            );
        }
    }

    #[test]
    fn two_sections_of_the_same_host_are_both_taken_and_given_back() {
        let content = "Host a\n\tUser u\nHost b\n\tUser b\nHost a\n\tPort 22\n";
        let claimed = claim_originals(content, &names(&["a"]));
        assert_eq!(claimed.matches("#hubergamof-off# Host a").count(), 2);
        assert!(claimed.contains("\nHost b\n\tUser b\n"));
        assert_eq!(claim_originals(&claimed, &names(&[])), content);
    }

    #[test]
    fn a_section_that_ssh_would_lose_something_of_is_never_taken() {
        for body in [
            "\tIdentityFile k1\n\tIdentityFile k2\n",
            "\tForwardAgent /tmp/agent.sock\n",
            "\tPort ssh\n",
        ] {
            let content = format!("Host a\n{body}");
            assert_eq!(claim_originals(&content, &names(&["a"])), content, "{body}");
        }
    }

    #[test]
    fn a_marked_section_that_is_not_a_host_is_not_given_back() {
        let content = "#hubergamof-off# Match all\n#hubergamof-off# \tUser m\n";
        assert_eq!(claim_originals(content, &names(&[])), content);
    }
}
