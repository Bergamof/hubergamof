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

if [[ $failures -ne 0 ]]; then
	echo "$failures test(s) en échec." >&2
	exit 1
fi
echo "Scripts de contrôle : tous les tests passent."
