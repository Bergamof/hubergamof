#!/usr/bin/env bash
# Écrit en Markdown la note d'une release : commits regroupés par type
# (Conventional Commits), instructions d'installation, lien de comparaison.
#
# Usage : release-notes.sh <version> <commit> [<tag précédent>]
# Variables : GITHUB_SERVER_URL, GITHUB_REPOSITORY (liens de PR et de comparaison).
set -euo pipefail

if (($# < 2 || $# > 3)); then
	echo "Usage : $(basename "$0") <version> <commit> [<tag précédent>]" >&2
	exit 2
fi
version="$1"
target="$2"
previous_tag="${3:-}"

server="${GITHUB_SERVER_URL:-https://github.com}"
repository="${GITHUB_REPOSITORY:-Bergamof/hubergamof}"
repo_url="$server/$repository"
readonly SUBJECT_PATTERN='^([a-z]+)(\([^)]*\))?(!)?: (.*)$'

range="$target"
[[ -z "$previous_tag" ]] || range="$previous_tag..$target"

breaking=() features=() fixes=() performance=() others=()

# Sujet publié tel quel, sauf : `(#18)` devient un lien vers la PR, et `@nom` est
# mis en code pour ne notifier personne.
# shellcheck disable=SC2016  # les backticks sont du Markdown littéral
link_pull_requests() {
	sed -E -e "s|\(#([0-9]+)\)|([#\1]($repo_url/pull/\1))|g" \
		-e 's/(^|[^`[:alnum:]])@([A-Za-z0-9_-]+)/\1`@\2`/g' <<<"$1"
}

classify() {
	local commit="$1" subject body type scope description entry
	subject="$(git log -1 --format=%s "$commit")"
	body="$(git log -1 --format=%b "$commit")"
	if [[ "$subject" =~ $SUBJECT_PATTERN ]]; then
		type="${BASH_REMATCH[1]}"
		scope="${BASH_REMATCH[2]}"
		description="${BASH_REMATCH[4]}"
		scope="${scope#(}"
		scope="${scope%)}"
		entry="${scope:+**$scope** : }$(link_pull_requests "$description")"
		if [[ -n "${BASH_REMATCH[3]}" ]] || grep -qE '^BREAKING[ -]CHANGE:' <<<"$body"; then
			breaking+=("$entry")
			return
		fi
		case "$type" in
		feat) features+=("$entry") ;;
		fix) fixes+=("$entry") ;;
		perf) performance+=("$entry") ;;
		*) others+=("$type : $entry") ;;
		esac
	else
		others+=("$(link_pull_requests "$subject")")
	fi
}

print_section() { # print_section <titre> <entrées…>
	local title="$1"
	shift
	(($# > 0)) || return 0
	printf '### %s\n\n' "$title"
	printf -- '- %s\n' "$@"
	printf '\n'
}

while IFS= read -r commit; do
	classify "$commit"
done < <(git rev-list --reverse "$range")

printf '## HuBergamof %s\n\n' "$version"
print_section "Changements incompatibles" "${breaking[@]}"
print_section "Nouveautés" "${features[@]}"
print_section "Corrections" "${fixes[@]}"
print_section "Performances" "${performance[@]}"
print_section "Autres changements" "${others[@]}"

cat <<INSTALL
### Installation

Ubuntu 24.04 ou plus, Debian 12 ou plus :

\`\`\`bash
sudo apt install ./hubergamof_${version}_amd64.deb
\`\`\`

Manjaro, Arch Linux :

\`\`\`bash
sudo pacman -U hubergamof-${version}-1-x86_64.pkg.tar.zst
\`\`\`

Le paquet installe la règle udev, active le service utilisateur et le démarre
dans les sessions graphiques ouvertes. Docker (module Docker) est une dépendance
optionnelle : l'utilisateur doit appartenir au groupe \`docker\`. Les sommes
SHA-256 des fichiers sont dans \`SHA256SUMS\`.

INSTALL

if [[ -n "$previous_tag" ]]; then
	printf '**Comparaison complète** : %s/compare/%s...v%s\n' "$repo_url" "$previous_tag" "$version"
fi
