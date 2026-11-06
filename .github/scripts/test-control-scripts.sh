#!/usr/bin/env bash
# Tests de check-node-version.sh et check-branch-name.sh (cas passants et refusés).
#
# Usage : test-control-scripts.sh
set -uo pipefail

here="$(dirname "${BASH_SOURCE[0]}")"
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
	printf 'set -euo pipefail\nsource "$1"\ncheck_pactl\n' >"$dir/runner.sh"
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

if [[ $failures -ne 0 ]]; then
	echo "$failures test(s) en échec." >&2
	exit 1
fi
echo "Scripts de contrôle : tous les tests passent."
