#!/usr/bin/env bash
# Tests des scripts de contrôle et de release (cas passants et refusés) :
# check-node-version, check-branch-name, check_pactl, next-version, release-notes,
# wait-for-ci (verdict), set-version, lanceur de bureau (install-desktop.sh et
# sa suppression par uninstall.sh).
#
# Usage : test-control-scripts.sh
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
failures=0

expect() { # expect <ok|ko> <script> <argument> <libellé>
	local expected="$1" script="$2" argument="$3" label="$4" status=0
	"$here/$script" "$argument" >/dev/null 2>&1 || status=$?
	if { [[ "$expected" == ok && $status -eq 0 ]] || [[ "$expected" == ko && $status -eq 1 ]]; }; then
		return
	fi
	echo "ÉCHEC : $label (attendu $expected, code $status)" >&2
	failures=$((failures + 1))
}

minimum="$(sed -nE 's/.*"node": *">=([0-9.]+)[ "].*/\1/p' "$here/../../ui/package.json" | head -1)"
expect ok check-node-version.sh "v$minimum" "Node au minimum"
expect ok check-node-version.sh "v24.99.0" "Node plus récent"
expect ko check-node-version.sh "v24.11.1" "Node 24.11.1 (Corepack 0.34.2)"
expect ko check-node-version.sh "v22.23.4" "Node 22"

expect ok check-branch-name.sh main "main"
expect ok check-branch-name.sh dependabot/cargo/serde-1.0.1 "Dependabot"
expect ok check-branch-name.sh fix/node-minimum-version "type/slug"
expect ok check-branch-name.sh claude/fix-node-minimum-version "préfixe de l'outil + type-slug"
expect ok check-branch-name.sh claude/feat/clock-seconds "préfixe + type/slug"
expect ko check-branch-name.sh claude/youthful-dijkstra-nvx2dj "nom aléatoire de l'outil"
expect ko check-branch-name.sh claude/amazing-clarke-pfm4h5 "autre nom aléatoire"
expect ko check-branch-name.sh fix/bug "un seul mot"
expect ko check-branch-name.sh feat/update "un seul mot"
expect ko check-branch-name.sh wip "wip"
expect ko check-branch-name.sh test "test"
expect ko check-branch-name.sh feature/add-clock "type inconnu"
expect ko check-branch-name.sh fix/Node-Version "majuscules"
expect ko check-branch-name.sh fix/node_version "tiret bas"

# check_pactl (packaging/common.sh) : avertit sans jamais interrompre l'installation.
# PATH réduit à des liens vers les seuls outils nécessaires, pour qu'un vrai pactl
# du poste ne fausse pas le test.
expect_pactl() { # expect_pactl <ok|warn> <corps du faux pactl, ou "absent"> <libellé>
	local expected="$1" body="$2" label="$3" dir status=0 output
	dir="$(mktemp -d)"
	ln -s "$(command -v dirname)" "$dir/dirname"
	ln -s "$(command -v head)" "$dir/head"
	cat >"$dir/runner.sh" <<'RUNNER'
set -euo pipefail
source "$1"
check_pactl
RUNNER
	if [[ "$body" != absent ]]; then
		printf '#!/bin/sh\n%s\n' "$body" >"$dir/pactl"
		chmod +x "$dir/pactl"
	fi
	output="$(PATH="$dir" "$BASH" "$dir/runner.sh" "$here/../../packaging/common.sh" 2>&1)" || status=$?
	rm -rf "$dir"
	if [[ $status -ne 0 ]]; then
		echo "ÉCHEC : $label (l'installation s'arrêterait, code $status)" >&2
		failures=$((failures + 1))
	elif { [[ "$expected" == warn && "$output" != *Attention* ]] || [[ "$expected" == ok && -n "$output" ]]; }; then
		echo "ÉCHEC : $label (attendu $expected, sortie : $output)" >&2
		failures=$((failures + 1))
	fi
}

expect_pactl warn absent "pactl absent"
expect_pactl warn 'echo "pactl 15.0"' "pactl 15.0"
expect_pactl ok 'echo "pactl 16.0"' "pactl 16.0 (limite)"
expect_pactl ok 'echo "pactl 16.1-ubuntu"' "pactl 16.1"
expect_pactl ok 'echo "pactl 17.0-rc1"' "pactl 17.0-rc1"
expect_pactl warn 'echo "pactl 08.0"' "version à zéro initial (pas d'octal)"
expect_pactl warn 'echo "n importe quoi"' "sortie illisible"
expect_pactl warn 'exit 1' "pactl en échec (n'interrompt pas l'installation)"

# next-version.sh : dépôt temporaire, commits et tags choisis par chaque cas.
# expect_version <attendu | "rien" | "usage"> <libellé> [options…] -- <sujets des commits…>
# Un sujet peut porter un corps après « || ». Le dépôt part d'un tag v1.2.3 sauf
# si le libellé commence par « sans tag ».
expect_version() {
	local expected="$1" label="$2" flags=() subjects=() output status=0 repo subject
	shift 2
	while [[ "${1:-}" != -- ]]; do flags+=("$1"); shift; done
	shift
	subjects=("$@")
	repo="$(mktemp -d)"
	(
		cd "$repo" || exit
		git init -q
		git config user.email test@example.invalid
		git config user.name test
		git config commit.gpgsign false
		git commit -q --allow-empty -m "chore: initial"
		[[ "$label" == "sans tag"* ]] || git tag v1.2.3
		for subject in "${subjects[@]}"; do
			if [[ "$subject" == *"||"* ]]; then
				git commit -q --allow-empty -m "${subject%%||*}" -m "${subject#*||}"
			else
				git commit -q --allow-empty -m "$subject"
			fi
		done
	)
	output="$(cd "$repo" && "$here/next-version.sh" "${flags[@]}" 2>/dev/null)" || status=$?
	rm -rf "$repo"
	if [[ "$expected" == rien ]]; then
		[[ $status -eq 3 ]] && return
	elif [[ "$expected" == usage ]]; then
		[[ $status -eq 2 ]] && return
	elif [[ $status -eq 0 && "$output" == "$expected" ]]; then
		return
	fi
	echo "ÉCHEC : $label (attendu $expected, obtenu « $output », code $status)" >&2
	failures=$((failures + 1))
}

expect_version 1.0.0 "sans tag : première version" -- "chore: rien"
expect_version 1.0.0 "sans tag : même avec un fix" -- "fix: x"
expect_version 1.2.4 "fix : patch" -- "fix(ui): x (#1)"
expect_version 1.2.4 "perf : patch" -- "perf: x"
expect_version 1.3.0 "feat : mineur" -- "feat: x"
expect_version 1.3.0 "feat l'emporte sur fix" -- "fix: a" "feat: b" "fix: c"
expect_version 2.0.0 "! : majeur" -- "feat!: x" "fix: y"
expect_version 2.0.0 "! avec portée : majeur" -- "fix(api)!: x"
expect_version 2.0.0 "BREAKING CHANGE dans le corps" -- "fix: x||BREAKING CHANGE: changement"
expect_version 2.0.0 "BREAKING-CHANGE dans le corps" -- "feat: x||détails

BREAKING-CHANGE: changement"
expect_version rien "seulement chore/ci/docs/refactor" -- "chore: a" "ci: b" "docs: c" "refactor: d"
expect_version rien "aucun commit depuis le tag" --
expect_version rien "sujet hors convention" -- "Update stuff"
expect_version rien "auto explicite : comme sans option" --bump auto -- "chore: a"
expect_version 1.3.0 "auto explicite avec un feat" --bump auto -- "feat: a"
expect_version 1.2.4 "patch forcé" --bump patch -- "chore: a" "refactor: b"
expect_version 1.3.0 "mineur forcé sans changement publiable" --bump minor -- "docs: a"
expect_version 2.0.0 "majeur forcé sans changement publiable" --bump major -- "chore: a"
expect_version 1.3.0 "mineur forcé au-dessus d'un fix" --bump minor -- "fix: a"
expect_version 2.0.0 "majeur forcé au-dessus d'un feat" --bump major -- "feat: a" "fix: b"
expect_version 1.3.0 "le patch forcé ne rétrograde pas un feat" --bump patch -- "feat: a"
expect_version 2.0.0 "le mineur forcé ne rétrograde pas un changement incompatible" --bump minor -- "feat!: a"
expect_version rien "forçage sans aucun commit" --bump major --
expect_version 1.0.0 "sans tag : le forçage garde la première version" --bump major -- "feat: a"
expect_version usage "niveau inconnu" --bump huge -- "feat: a"
expect_version usage "--bump sans niveau" --bump -- "feat: a"
expect_version usage "ancienne option --force-patch" --force-patch -- "chore: a"

# Tag de pré-version ignoré : la base reste v1.2.3.
prerelease_repo="$(mktemp -d)"
(
	cd "$prerelease_repo" || exit
	git init -q
	git config user.email test@example.invalid
	git config user.name test
	git commit -q --allow-empty -m "chore: initial"
	git tag v1.2.3
	git commit -q --allow-empty -m "fix: a"
	git tag v2.0.0-rc1
)
if [[ "$(cd "$prerelease_repo" && "$here/next-version.sh" 2>/dev/null)" != 1.2.4 ]]; then
	echo "ÉCHEC : un tag de pré-version ne doit pas servir de base" >&2
	failures=$((failures + 1))
fi
if [[ "$(cd "$prerelease_repo" && "$here/next-version.sh" --previous-tag)" != v1.2.3 ]]; then
	echo "ÉCHEC : --previous-tag doit donner v1.2.3" >&2
	failures=$((failures + 1))
fi
rm -rf "$prerelease_repo"
no_tag_repo="$(mktemp -d)"
(cd "$no_tag_repo" && git init -q && git -c user.email=t@e.invalid -c user.name=t commit -q --allow-empty -m "chore: a")
if [[ -n "$(cd "$no_tag_repo" && "$here/next-version.sh" --previous-tag)" ]]; then
	echo "ÉCHEC : --previous-tag sans tag doit être vide" >&2
	failures=$((failures + 1))
fi
rm -rf "$no_tag_repo"

# release-notes.sh : regroupement par type et liens de PR.
notes_repo="$(mktemp -d)"
(
	cd "$notes_repo" || exit
	git init -q
	git config user.email test@example.invalid
	git config user.name test
	git commit -q --allow-empty -m "chore: initial"
	git tag v1.0.0
	git commit -q --allow-empty -m "feat(docker): nouvelle page (#7)"
	git commit -q --allow-empty -m "fix: crash au démarrage (#8)"
	git commit -q --allow-empty -m "feat!: nouveau format"
	git commit -q --allow-empty -m "ci: étape de plus"
	git commit -q --allow-empty -m "docs: merci @someone et @org-team"
)
notes="$(cd "$notes_repo" && GITHUB_SERVER_URL=https://example.invalid GITHUB_REPOSITORY=o/r \
	"$here/release-notes.sh" 1.1.0 HEAD v1.0.0)"
rm -rf "$notes_repo"
# shellcheck disable=SC2016  # backticks Markdown littéraux
for fragment in '## HuBergamof 1.1.0' '### Changements incompatibles' '- nouveau format' \
	'### Nouveautés' '- **docker** : nouvelle page ([#7](https://example.invalid/o/r/pull/7))' \
	'### Corrections' '- crash au démarrage ([#8](https://example.invalid/o/r/pull/8))' \
	'### Autres changements' '- ci : étape de plus' '- docs : merci `@someone` et `@org-team`' 'hubergamof_1.1.0_amd64.deb' \
	'hubergamof-1.1.0-1-x86_64.pkg.tar.zst' 'compare/v1.0.0...v1.1.0'; do
	if [[ "$notes" != *"$fragment"* ]]; then
		echo "ÉCHEC : release-notes.sh, « $fragment » absent" >&2
		failures=$((failures + 1))
	fi
done
if [[ "$notes" == *'### Performances'* ]]; then
	echo "ÉCHEC : release-notes.sh, section vide affichée" >&2
	failures=$((failures + 1))
fi

# wait-for-ci.sh --verdict : le run le plus récent du commit fait foi.
expect_verdict() { # expect_verdict <attendu> <JSON> <libellé>
	local got
	got="$("$here/wait-for-ci.sh" --verdict <<<"$2")"
	if [[ "$got" != "$1" ]]; then
		echo "ÉCHEC : $3 (attendu $1, obtenu $got)" >&2
		failures=$((failures + 1))
	fi
}
expect_verdict missing '{"workflow_runs":[]}' "aucun run"
expect_verdict success '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"success"}]}' "run vert"
expect_verdict failure '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"failure"}]}' "run rouge"
expect_verdict failure '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"cancelled"}]}' "run annulé"
expect_verdict pending '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"in_progress","conclusion":null}]}' "run en cours"
expect_verdict pending '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"queued","conclusion":null}]}' "run en file"
expect_verdict success '{"workflow_runs":[{"event":"push","created_at":"2026-01-02T00:00:00Z","status":"completed","conclusion":"success"},{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"failure"}]}' "le plus récent est vert"
expect_verdict failure '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"success"},{"event":"push","created_at":"2026-01-02T00:00:00Z","status":"completed","conclusion":"failure"}]}' "le plus récent est rouge"

expect_verdict success '{"workflow_runs":[{"event":"schedule","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"success"}]}' "run planifié accepté"
expect_verdict missing '{"workflow_runs":[{"event":"pull_request","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"success"}]}' "run de PR ignoré"
expect_verdict success '{"workflow_runs":[{"event":"push","created_at":"2026-01-01T00:00:00Z","status":"completed","conclusion":"success"},{"event":"pull_request","created_at":"2026-01-02T00:00:00Z","status":"completed","conclusion":"failure"}]}' "un run de PR plus récent ne compte pas"

# set-version.sh : Cargo.toml et Cargo.lock, rien d'autre.
version_dir="$(mktemp -d)"
cat >"$version_dir/Cargo.toml" <<'TOML'
[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
serde = { version = "0.1.0" }
TOML
cat >"$version_dir/Cargo.lock" <<'LOCK'
[[package]]
name = "hubergamof"
version = "0.1.0"

[[package]]
name = "hubergamof-modules"
version = "0.1.0"

[[package]]
name = "hubergamof-protocol-extra-lib"
version = "0.1.0"

[[package]]
name = "serde"
version = "0.1.0"
LOCK
(cd "$version_dir" && "$here/set-version.sh" 3.4.5 >/dev/null)
toml_versions="$(grep -c '"3.4.5"' "$version_dir/Cargo.toml")"
lock_versions="$(grep -c 'version = "3.4.5"' "$version_dir/Cargo.lock")"
untouched="$(grep -c 'version = "0.1.0"' "$version_dir/Cargo.lock")"
if [[ "$toml_versions" != 1 || "$lock_versions" != 3 || "$untouched" != 1 ]] \
	|| ! grep -q 'serde = { version = "0.1.0" }' "$version_dir/Cargo.toml"; then
	echo "ÉCHEC : set-version.sh (toml=$toml_versions, lock=$lock_versions, intacts=$untouched)" >&2
	failures=$((failures + 1))
fi
if (cd "$version_dir" && "$here/set-version.sh" "1.0" >/dev/null 2>&1); then
	echo "ÉCHEC : set-version.sh accepte une version invalide" >&2
	failures=$((failures + 1))
fi
rm -rf "$version_dir"

# hooks.sh (scripts de maintenance des paquets) : faux loginctl/systemctl qui
# journalisent leurs appels. Seule une session graphique d'un compte humain doit
# être touchée ; aucune commande ne doit échouer.
hooks_dir="$(mktemp -d)"
mkdir "$hooks_dir/run-systemd"
cat >"$hooks_dir/loginctl" <<'FAKE'
#!/bin/sh
printf '%s\n' "0 root" "999 sysuser" "1000 alice" "1001 bob"
FAKE
cat >"$hooks_dir/systemctl" <<'FAKE'
#!/bin/sh
printf '%s\n' "$*" >>"$HOOKS_LOG"
case "$*" in
*bob@.host*is-active*) exit 3 ;;
*alice@.host*restart*) [ -z "$HOOKS_FAIL_RESTART" ] || exit 1 ;;
esac
FAKE
chmod +x "$hooks_dir/loginctl" "$hooks_dir/systemctl"
run_hook() { # run_hook <fonction> [variables d'environnement…] : journal dans $hooks_dir/log
	local function="$1"
	shift
	: >"$hooks_dir/log"
	env "$@" HOOKS_LOG="$hooks_dir/log" PATH="$hooks_dir:/usr/bin:/bin" \
		HUBERGAMOF_SYSTEMD_DIR="$hooks_dir/run-systemd" \
		sh -c ". '$here/../../packaging/package/hooks.sh'; $function" 2>"$hooks_dir/stderr"
}
expect_log() { # expect_log <fragment attendu> <libellé>
	grep -qF -- "$1" "$hooks_dir/log" && return
	echo "ÉCHEC : hooks.sh, $2 ($1)" >&2
	failures=$((failures + 1))
}
expect_no_log() { # expect_no_log <fragment interdit> <libellé>
	grep -qF -- "$1" "$hooks_dir/log" || return 0
	echo "ÉCHEC : hooks.sh, $2 ($1)" >&2
	failures=$((failures + 1))
}
if ! run_hook hubergamof_post_install HOOKS_FAIL_RESTART=; then
	echo "ÉCHEC : hooks.sh, post_install ne doit pas échouer" >&2
	failures=$((failures + 1))
fi
expect_log '--global enable hubergamof.service' "activation globale"
expect_log '--user --machine=alice@.host --quiet is-active graphical-session.target' "session testée"
expect_log '--user --machine=alice@.host daemon-reload' "rechargement de la session"
expect_log '--user --machine=alice@.host restart hubergamof.service' "démarrage dans la session"
expect_no_log '--machine=bob@.host restart' "pas de démarrage sans session graphique"
expect_no_log '--machine=root@.host' "root ignoré"
expect_no_log '--machine=sysuser@.host' "compte système ignoré"
if run_hook hubergamof_post_install HOOKS_FAIL_RESTART=1 && ! grep -q 'a échoué pour alice' "$hooks_dir/stderr"; then
	echo "ÉCHEC : hooks.sh, l'échec d'un démarrage doit être signalé sans interrompre" >&2
	failures=$((failures + 1))
fi
run_hook "hubergamof_post_install upgrade" HOOKS_FAIL_RESTART=
expect_log '--user --machine=alice@.host try-restart hubergamof.service' "mise à jour : redémarrage seulement là où il tourne"
expect_no_log ' restart hubergamof.service' "mise à jour : pas de démarrage forcé"
run_hook hubergamof_pre_remove HOOKS_FAIL_RESTART=
expect_log '--user --machine=alice@.host stop hubergamof.service' "arrêt avant retrait"
expect_log '--global disable hubergamof.service' "désactivation globale"
rm -rf "$hooks_dir/run-systemd"
run_hook hubergamof_post_install HOOKS_FAIL_RESTART=
if [[ -s "$hooks_dir/log" ]]; then
	echo "ÉCHEC : hooks.sh, sans systemd en cours d'exécution rien ne doit être appelé" >&2
	failures=$((failures + 1))
fi
rm -rf "$hooks_dir"

# Lanceur de bureau : clé Exec échappée pour un chemin piégé, fichiers posés
# exactement ceux que liste desktop_entry_files (que uninstall.sh retire).
desktop_dir="$(mktemp -d)"
cat >"$desktop_dir/runner.sh" <<'RUNNER'
set -euo pipefail
source "$1/packaging/desktop/install-desktop.sh"
install_desktop_entry "$2" "$3/share"
grep '^Exec=' "$3/share/$DESKTOP_ENTRY_TARGET" >"$3/exec"
desktop_entry_files | sort >"$3/listed"
(cd "$3/share" && find . -type f | sed 's|^\./||' | sort) >"$3/installed"
RUNNER
# shellcheck disable=SC2016  # chemin et clé littéraux, rien à développer
tricky_binary='/home/a b/$x`y"z%w\q/hubergamof'
# shellcheck disable=SC2016
expected_exec='Exec="/home/a b/\\$x\\`y\\"z%%w\\\\q/hubergamof" open'
if ! "$BASH" "$desktop_dir/runner.sh" "$here/../.." "$tricky_binary" "$desktop_dir"; then
	echo "ÉCHEC : install-desktop.sh, pose du lanceur" >&2
	failures=$((failures + 1))
elif [[ "$(<"$desktop_dir/exec")" != "$expected_exec" ]]; then
	echo "ÉCHEC : install-desktop.sh, clé Exec « $(<"$desktop_dir/exec") », attendue « $expected_exec »" >&2
	failures=$((failures + 1))
elif ! cmp -s "$desktop_dir/listed" "$desktop_dir/installed"; then
	echo "ÉCHEC : install-desktop.sh, desktop_entry_files ne liste pas les fichiers posés" >&2
	failures=$((failures + 1))
elif command -v desktop-file-validate >/dev/null \
	&& ! desktop-file-validate "$desktop_dir/share/applications/hubergamof.desktop" >/dev/null; then
	echo "ÉCHEC : install-desktop.sh, lanceur refusé par desktop-file-validate" >&2
	failures=$((failures + 1))
fi
# uninstall.sh : remove_launcher retire tout ce qui a été posé.
cat >"$desktop_dir/remove.sh" <<'REMOVE'
set -euo pipefail
source "$1/packaging/desktop/install-desktop.sh"
DATA_DIR="$2/share"
info() { :; }
eval "$(sed -n '/^remove_launcher() {/,/^}/p' "$1/packaging/uninstall.sh")"
remove_launcher
REMOVE
if ! "$BASH" "$desktop_dir/remove.sh" "$here/../.." "$desktop_dir" \
	|| [[ -n "$(find "$desktop_dir/share" -type f)" ]]; then
	echo "ÉCHEC : uninstall.sh, remove_launcher laisse des fichiers du lanceur" >&2
	failures=$((failures + 1))
fi
rm -rf "$desktop_dir"

if [[ $failures -ne 0 ]]; then
	echo "$failures test(s) en échec." >&2
	exit 1
fi
echo "Scripts de contrôle : tous les tests passent."
