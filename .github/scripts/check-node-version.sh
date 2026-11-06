#!/usr/bin/env bash
# Refuse un Node plus ancien que le minimum de `engines.node` (ui/package.json).
#
# pnpm 12 n'a plus de `bin/pnpm.cjs` : les Corepack livrés avec Node < 24.12
# (Corepack < 0.34.5) échouent avec « Cannot find module …/pnpm.cjs » avant
# même de lancer pnpm, qui ne peut donc pas appliquer `engines` lui-même. Ce
# contrôle donne le message utile à la place de cette trace.
#
# Usage : check-node-version.sh [version-de-node]   (défaut : `node --version`)
set -euo pipefail

package_json="$(dirname "${BASH_SOURCE[0]}")/../../ui/package.json"
minimum="$(sed -nE 's/.*"node": *">=([0-9]+(\.[0-9]+){0,2})[ "].*/\1/p' "$package_json" | head -1)"
[[ -n "$minimum" ]] || { echo "Minimum de engines.node introuvable dans $package_json." >&2; exit 2; }

current="${1:-$(node --version)}"
current="${current#v}"

# `sort -V` : le plus petit des deux vient en premier.
lowest="$(printf '%s\n%s\n' "$minimum" "$current" | sort -V | head -1)"
if [[ "$lowest" != "$minimum" ]]; then
	cat >&2 <<MESSAGE
Node $current est trop ancien : Node >= $minimum requis (Corepack >= 0.34.5 pour pnpm 12).
Installer une version récente puis effacer le cache de pnpm laissé par l'échec :
  nvm install && nvm use
  rm -rf ~/.cache/node/corepack/v1/pnpm
  corepack enable pnpm
MESSAGE
	exit 1
fi
echo "Node $current (minimum $minimum)."
