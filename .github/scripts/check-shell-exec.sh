#!/usr/bin/env bash
# Refuse tout lancement de shell (`sh`, `bash`, `/bin/sh`…) dans le code Rust
# de production : une valeur de configuration passée à `sh -c` permettrait
# d'injecter des commandes. Les modules lancent les programmes directement
# (`Command::new(programme).args(…)`), sans interpréteur intermédiaire.
#
# Repère tout littéral chaîne désignant un shell (argument de `Command::new`,
# constante, champ, `cmd` d'un `docker exec`…), hors commentaires et hors code
# de test : fichiers `tests.rs`, dossiers `tests/`, fichiers marqués
# `#![cfg(test)]` et éléments précédés de `#[cfg(test)]`.
#
# Usage : check-shell-exec.sh [dossier…]   (défaut : crates)
set -euo pipefail

roots=("$@")
[[ ${#roots[@]} -gt 0 ]] || roots=(crates)

mapfile -d '' sources < <(
	find "${roots[@]}" -type f -name '*.rs' -path '*/src/*' \
		! -name tests.rs ! -path '*/tests/*' -print0 | sort -z
)
[[ ${#sources[@]} -gt 0 ]] || { echo 'Aucun fichier Rust à analyser.'; exit 0; }

# Le comptage d'accolades d'un élément `#[cfg(test)]` ignore les chaînes, les
# caractères et les commentaires de ligne ; un élément sans corps (`mod x;`,
# `use …;`) se termine au premier `;`.
awk '
	FNR == 1 { pending_test = 0; depth = 0; in_test = 0; skip_file = 0 }
	skip_file { next }
	{
		code = $0
		gsub(/"([^"\\]|\\.)*"/, "\"\"", code)
		gsub(/\x27([^\x27\\]|\\.)\x27/, "\x27\x27", code)
		sub(/\/\/.*/, "", code)
	}
	code ~ /^[[:space:]]*#!\[cfg\(test\)\]/ { skip_file = 1; next }
	!in_test && code ~ /#\[cfg\((all\()?test[,)]/ { pending_test = 1 }
	pending_test && !in_test {
		if (index(code, "{")) { in_test = 1; pending_test = 0; depth = 0 }
		else if (code ~ /;[[:space:]]*$/) { pending_test = 0; next }
	}
	in_test {
		opened = gsub(/\{/, "{", code); closed = gsub(/\}/, "}", code)
		depth += opened - closed
		if (depth <= 0) in_test = 0
		next
	}
	{
		line = $0
		sub(/^[[:space:]]*\/\/.*/, "", line)
		if (line ~ /"(\/usr)?(\/bin\/)?(sh|bash|dash|zsh|ksh|fish)"/) {
			printf "%s:%d: lancement de shell interdit hors tests : %s\n", FILENAME, FNR, $0 > "/dev/stderr"
			found = 1
		}
	}
	END {
		if (found) {
			print "Lancer le programme directement (Command::new(programme).args(…)), jamais via un shell : risque d\047injection de commandes depuis la configuration." > "/dev/stderr"
			exit 1
		}
	}
' "${sources[@]}" && echo "Aucun lancement de shell hors tests (${#sources[@]} fichiers analysés)."
