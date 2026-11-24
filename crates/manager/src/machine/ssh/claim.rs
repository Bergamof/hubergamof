//! Commenting out the `Host` sections that HuBergamof took over from
//! `~/.ssh/config`, so that switching a host off really switches it off: the
//! file of HuBergamof comes first and wins, but a host that is off must not
//! come back from the section it was taken from.
//!
//! The rule has no memory: a line commented out by HuBergamof carries
//! [`CLAIMED_MARKER`], which says what to give back once the host is no longer
//! in the configuration. Pure text functions.

use std::collections::HashSet;

use super::sections::{CLAIMED_MARKER, Header, lines_of, option_of, sections};

/// Options that make `ssh` run a program or load more configuration. A
/// section holding one is never given back by HuBergamof: a marker can come
/// from somewhere else (dotfiles shared by two computers), and it must not
/// make HuBergamof switch on what it refuses to write. Such a section stays
/// commented out, and the person can uncomment it.
const OPTIONS_THAT_RUN_A_PROGRAM: [&str; 10] = [
    "proxycommand",
    "localcommand",
    "permitlocalcommand",
    "remotecommand",
    "knownhostscommand",
    "match",
    "include",
    "pkcs11provider",
    "securitykeyprovider",
    "identityagent",
];

/// The alias of a section that is one host: a single name, no joker.
fn single_alias(patterns: &[String]) -> Option<&str> {
    match patterns {
        [alias] if !alias.contains(['*', '?', '!']) => Some(alias),
        _ => None,
    }
}

/// Whether every non-blank line of `lines` is commented out by HuBergamof,
/// holds nothing that runs a program, and has its quotes in pairs: what
/// HuBergamof could have commented out itself.
fn may_give_back(lines: &[&str]) -> bool {
    lines.iter().all(|line| {
        let text = line.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            return true;
        }
        let Some(inner) = text.strip_prefix(CLAIMED_MARKER) else {
            return false;
        };
        let runs_a_program = option_of(inner)
            .is_some_and(|(keyword, _)| OPTIONS_THAT_RUN_A_PROGRAM.contains(&keyword.as_str()));
        !runs_a_program && inner.matches('"').count() % 2 == 0
    })
}

/// `content` with the `Host` sections of the hosts of `aliases` commented out,
/// and the sections commented out earlier whose host is no longer in
/// `aliases` given back.
///
/// Only a section that is one host (a single name, no joker) is touched. A
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
        if !section.marked && named {
            for line in own {
                if line.trim().is_empty() {
                    claimed.push_str(line);
                } else {
                    claimed.push_str(CLAIMED_MARKER);
                    claimed.push_str(line);
                }
            }
        } else if section.marked && alias.is_some() && !named && may_give_back(own) {
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
    fn a_section_that_runs_a_program_is_never_given_back() {
        for option in [
            "ProxyCommand nc %h %p",
            "LocalCommand curl x | sh",
            "PermitLocalCommand yes",
            "KnownHostsCommand x",
            "Include other",
            "IdentityAgent /tmp/x",
        ] {
            let content = format!("#hubergamof-off# Host x\n#hubergamof-off# \t{option}\n");
            assert_eq!(claim_originals(&content, &names(&[])), content, "{option}");
        }
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
    fn a_marked_section_that_is_not_a_host_is_not_given_back() {
        let content = "#hubergamof-off# Match all\n#hubergamof-off# \tUser m\n";
        assert_eq!(claim_originals(content, &names(&[])), content);
    }
}
