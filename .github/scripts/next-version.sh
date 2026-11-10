#!/usr/bin/env bash
# Calcule le prochain numéro de version à partir des commits (Conventional
# Commits) créés depuis le dernier tag `vX.Y.Z`.
#
#   - aucun tag de version : 1.0.0 ;
#   - `!` après le type ou `BREAKING CHANGE:` dans le corps : majeur ;
#   - `feat` : mineur ; `fix` ou `perf` : patch ;
#   - aucun de ceux-là : code 3 (rien à publier), sauf `--force-patch` qui
#     publie un patch ; sans aucun commit depuis le tag, rien n'est publiable.
#
# Écrit la version (sans `v`) sur la sortie standard, le diagnostic sur la
# sortie d'erreur. S'exécute dans le dépôt courant, historique et tags complets.
#
# Avec --previous-tag, écrit seulement le dernier tag de version (rien s'il n'y en a pas).
#
# Usage : next-version.sh [--force-patch | --previous-tag] [<commit>]   (défaut : HEAD)
# Codes : 0 version calculée, 1 erreur, 2 usage, 3 rien à publier.
set -euo pipefail

readonly EXIT_NOTHING_TO_RELEASE=3
readonly FIRST_VERSION="1.0.0"
readonly TAG_GLOB='v[0-9]*.[0-9]*.[0-9]*'
readonly TAG_PATTERN='^v([0-9]+)\.([0-9]+)\.([0-9]+)$'
readonly SUBJECT_PATTERN='^([a-z]+)(\([^)]*\))?(!)?: '
readonly BREAKING_BODY_PATTERN='^BREAKING[ -]CHANGE:'

force_patch=false
print_previous_tag=false
target=HEAD
while (($# > 0)); do
	case "$1" in
	--force-patch) force_patch=true ;;
	--previous-tag) print_previous_tag=true ;;
	-*)
		echo "Usage : $(basename "$0") [--force-patch | --previous-tag] [<commit>]" >&2
		exit 2
		;;
	*) target="$1" ;;
	esac
	shift
done

# La plus haute version publiée parmi les tags atteignables depuis la cible,
# hors pré-versions (`v2.0.0-rc1`).
last_release_tag() {
	local tag
	while IFS= read -r tag; do
		if [[ "$tag" =~ $TAG_PATTERN ]]; then
			printf '%s' "$tag"
			return 0
		fi
	done < <(git tag --merged "$target" --list "$TAG_GLOB" --sort=-v:refname)
	return 1
}

# Plus haut niveau de changement des commits : major | minor | patch | none.
highest_bump() {
	local range="$1" level=none subject body commit
	while IFS= read -r commit; do
		subject="$(git log -1 --format=%s "$commit")"
		body="$(git log -1 --format=%b "$commit")"
		if [[ "$subject" =~ $SUBJECT_PATTERN ]]; then
			local type="${BASH_REMATCH[1]}" bang="${BASH_REMATCH[3]}"
			if [[ -n "$bang" ]]; then
				printf 'major'
				return
			fi
			case "$type" in
			feat) [[ "$level" == major ]] || level=minor ;;
			fix | perf) [[ "$level" == none ]] && level="patch" ;;
			esac
		fi
		if grep -qE "$BREAKING_BODY_PATTERN" <<<"$body"; then
			printf 'major'
			return
		fi
	done < <(git rev-list "$range")
	printf '%s' "$level"
}

if [[ "$print_previous_tag" == true ]]; then
	last_release_tag || true
	exit 0
fi

if ! tag="$(last_release_tag)"; then
	echo "Aucun tag de version : première version $FIRST_VERSION." >&2
	printf '%s\n' "$FIRST_VERSION"
	exit 0
fi

commit_count="$(git rev-list --count "$tag..$target")"
if ((commit_count == 0)); then
	echo "Aucun commit depuis $tag : rien à publier." >&2
	exit "$EXIT_NOTHING_TO_RELEASE"
fi

[[ "$tag" =~ $TAG_PATTERN ]]
major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"

bump="$(highest_bump "$tag..$target")"
if [[ "$bump" == none ]]; then
	if [[ "$force_patch" != true ]]; then
		echo "Aucun feat, fix, perf ni changement incompatible depuis $tag ($commit_count commit(s)) : rien à publier (--force-patch pour publier un patch)." >&2
		exit "$EXIT_NOTHING_TO_RELEASE"
	fi
	bump="patch"
	echo "Aucun changement publiable depuis $tag : patch forcé." >&2
fi

case "$bump" in
major) printf '%s.0.0\n' "$((major + 1))" ;;
minor) printf '%s.%s.0\n' "$major" "$((minor + 1))" ;;
patch) printf '%s.%s.%s\n' "$major" "$minor" "$((patch + 1))" ;;
esac
echo "Changement $bump depuis $tag ($commit_count commit(s))." >&2
