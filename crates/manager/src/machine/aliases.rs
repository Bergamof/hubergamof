//! Reading aliases from shell startup files, without running a shell, and
//! writing them as a file a shell can source.

use std::collections::HashMap;

use hubergamof_protocol::{ShellAlias, is_valid_alias_command, is_valid_alias_name};
use thiserror::Error;

use super::rc_block::outside_blocks;

/// The word that starts an alias definition.
const ALIAS_KEYWORD: &str = "alias";

/// Characters that end an unquoted word in a shell command line.
const WORD_TERMINATORS: [char; 6] = [';', '&', '|', '<', '>', '('];

/// First line of the managed file.
const MANAGED_HEADER: &str = "# Managed by HuBergamof, do not edit: changes are overwritten.\n";

/// An alias that cannot be written to a shell file safely.
#[derive(Debug, Error)]
#[error("alias `{0}` cannot be written to a shell file")]
pub struct UnwritableAlias(pub String);

/// The `alias name=command` definitions of a startup file, one per line,
/// as `(name, command)`: the last definition of a name wins, order of first
/// appearance. Lines that are not a plain, complete definition on their own
/// (comments, `unalias`, an `alias` inside another command, several
/// definitions, an unterminated quote, an invalid name) are ignored, and so
/// is the block HuBergamof keeps in the file.
pub(super) fn parse_aliases(content: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut position: HashMap<String, usize> = HashMap::new();
    for (name, command) in outside_blocks(content).filter_map(parse_line) {
        if let Some(&index) = position.get(&name) {
            found[index].1 = command;
        } else {
            position.insert(name.clone(), found.len());
            found.push((name, command));
        }
    }
    found
}

fn parse_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix(ALIAS_KEYWORD)?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let (name, value) = rest.trim_start().split_once('=')?;
    if !is_valid_alias_name(name) {
        return None;
    }
    let (command, tail) = parse_word(value)?;
    let tail = tail.trim();
    if !(tail.is_empty() || tail.starts_with('#')) || !is_valid_alias_command(&command) {
        return None;
    }
    Some((name.to_owned(), command))
}

/// The shell word at the start of `input` with its quotes removed, and what
/// follows it; `None` for an unterminated quote or escape.
fn parse_word(input: &str) -> Option<(String, &str)> {
    let mut word = String::new();
    let mut chars = input.char_indices().peekable();
    while let Some(&(index, c)) = chars.peek() {
        match c {
            '\'' => {
                chars.next();
                loop {
                    match chars.next()?.1 {
                        '\'' => break,
                        inside => word.push(inside),
                    }
                }
            }
            '"' => {
                chars.next();
                loop {
                    match chars.next()?.1 {
                        '"' => break,
                        '\\' => {
                            let escaped = chars.next()?.1;
                            if !matches!(escaped, '"' | '\\' | '$' | '`') {
                                word.push('\\');
                            }
                            word.push(escaped);
                        }
                        inside => word.push(inside),
                    }
                }
            }
            '\\' => {
                chars.next();
                word.push(chars.next()?.1);
            }
            c if c.is_whitespace() || WORD_TERMINATORS.contains(&c) => {
                return Some((word, &input[index..]));
            }
            plain => {
                word.push(plain);
                chars.next();
            }
        }
    }
    Some((word, ""))
}

/// `command` as a single-quoted shell word: nothing inside is expanded, a
/// quote is closed, escaped and reopened.
fn single_quoted(command: &str) -> String {
    format!("'{}'", command.replace('\'', r"'\''"))
}

/// The content of the managed file for the enabled `aliases`.
///
/// # Errors
/// [`UnwritableAlias`] for a name or command the contract would refuse: the
/// file is run by the shell, so nothing unchecked is written to it.
pub(super) fn render_managed_file(aliases: &[&ShellAlias]) -> Result<String, UnwritableAlias> {
    let mut lines = vec![MANAGED_HEADER.to_owned()];
    for alias in aliases {
        if !is_valid_alias_name(&alias.name) || !is_valid_alias_command(&alias.command) {
            return Err(UnwritableAlias(alias.name.clone()));
        }
        lines.push(format!(
            "{ALIAS_KEYWORD} {}={}\n",
            alias.name,
            single_quoted(&alias.command)
        ));
    }
    Ok(lines.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, command: &str) -> (String, String) {
        (name.to_owned(), command.to_owned())
    }

    #[test]
    fn the_three_forms_of_quoting_are_read() {
        let content = "alias a='ls -la'\nalias b=\"ls -la\"\nalias c=ls\n";
        assert_eq!(
            parse_aliases(content),
            [pair("a", "ls -la"), pair("b", "ls -la"), pair("c", "ls")]
        );
    }

    #[test]
    fn quotes_are_resolved_like_a_shell_does() {
        let content = [
            r"alias a='echo '\''hi'\'''",
            r#"alias b="echo \"hi\" \$HOME \\ \x""#,
            r"alias c=echo\ hi",
            r#"alias d='a'"b"c"#,
        ]
        .join("\n");
        assert_eq!(
            parse_aliases(&content),
            [
                pair("a", "echo 'hi'"),
                pair("b", r#"echo "hi" $HOME \ \x"#),
                pair("c", "echo hi"),
                pair("d", "abc"),
            ]
        );
    }

    #[test]
    fn indentation_comments_and_crlf_are_handled() {
        let content = "  alias a='x'  # why\n\talias b=y\r\n# alias c=z\n";
        assert_eq!(parse_aliases(content), [pair("a", "x"), pair("b", "y")]);
    }

    #[test]
    fn lines_that_are_not_a_plain_definition_are_ignored() {
        let content = [
            "unalias a",
            "echo alias a=b",
            "aliases=1",
            "alias",
            "alias -p",
            "alias a",
            "alias a=",
            "alias a=''",
            "alias 1a=x",
            "alias a b=x",
            "alias a='unterminated",
            "alias a=b c=d",
            "alias a=b && alias c=d",
            "alias a=b; ls",
            "alias a=b\\",
            "if x; then alias a=b; fi",
            "alias a='tab\there'",
        ]
        .join("\n");
        assert_eq!(parse_aliases(&content), []);
    }

    #[test]
    fn the_last_definition_of_a_name_wins() {
        let content = "alias a=1\nalias b=2\nalias a=3\n";
        assert_eq!(parse_aliases(content), [pair("a", "3"), pair("b", "2")]);
    }

    #[test]
    fn expansions_are_kept_as_written() {
        let content = "alias a='echo $(date) `id` ${HOME}'\n";
        assert_eq!(
            parse_aliases(content),
            [pair("a", "echo $(date) `id` ${HOME}")]
        );
    }

    #[test]
    fn the_managed_block_is_not_read() {
        let content = "alias a=1\n# >>> hubergamof aliases >>>\nalias b=2\n# <<< hubergamof aliases <<<\nalias c=3\n";
        assert_eq!(parse_aliases(content), [pair("a", "1"), pair("c", "3")]);
    }

    fn enabled(name: &str, command: &str) -> ShellAlias {
        ShellAlias {
            name: name.into(),
            command: command.into(),
            enabled: true,
        }
    }

    #[test]
    fn quotes_inside_a_command_are_escaped() {
        assert_eq!(single_quoted("it's"), r"'it'\''s'");
        assert_eq!(single_quoted("''"), r"''\'''\'''");
    }

    #[test]
    fn nothing_is_expanded_in_the_managed_file() {
        let aliases = [
            enabled("a", "echo $(touch /tmp/x) `id` $HOME \"q\" \\n"),
            enabled("b", "echo 'x'; rm -rf ~"),
        ];
        let file = render_managed_file(&aliases.iter().collect::<Vec<_>>()).unwrap();
        assert!(file.starts_with(MANAGED_HEADER));
        assert!(file.contains("alias a='echo $(touch /tmp/x) `id` $HOME \"q\" \\n'\n"));
        assert!(file.contains(r"alias b='echo '\''x'\''; rm -rf ~'"));
    }

    #[test]
    fn what_is_written_is_read_back_unchanged() {
        let aliases = [
            enabled("a", "echo $(date) `id` 'q' \"d\" \\ end"),
            enabled("b", "ls"),
        ];
        let file = render_managed_file(&aliases.iter().collect::<Vec<_>>()).unwrap();
        assert_eq!(
            parse_aliases(&file),
            [
                pair("a", "echo $(date) `id` 'q' \"d\" \\ end"),
                pair("b", "ls")
            ]
        );
    }

    #[test]
    fn an_alias_the_contract_would_refuse_is_not_written() {
        for bad in [enabled("a b", "ls"), enabled("a", "x\ny"), enabled("a", "")] {
            assert!(render_managed_file(&[&bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_real_shell_reads_the_managed_file_as_intended() {
        let command = "echo 'it'\"s\" $(touch SHOULD_NOT_EXIST) `id` $HOME \\ end";
        let dir = tempfile::tempdir().unwrap();
        let file = render_managed_file(&[&enabled("tricky", command)]).unwrap();
        let managed = dir.path().join("aliases.sh");
        std::fs::write(&managed, file).unwrap();
        let script = dir.path().join("show.sh");
        std::fs::write(&script, ". \"$1\"\nalias tricky\n").unwrap();
        let output = std::process::Command::new("sh")
            .current_dir(dir.path())
            .arg(&script)
            .arg(&managed)
            .output()
            .unwrap();
        let shown = String::from_utf8(output.stdout).unwrap();
        let line = shown.trim_start_matches("alias ").trim_end();
        assert_eq!(
            parse_aliases(&format!("alias {line}")),
            [pair("tricky", command)]
        );
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
    }
}
