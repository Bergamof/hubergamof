#!/usr/bin/env bash
# Fixe la version du workspace dans Cargo.toml et Cargo.lock (copie de travail
# de la CI uniquement : la version n'est jamais committée, elle vient du tag).
# Le binaire la rapporte via `--version`, les paquets via leur métadonnée.
#
# Usage : set-version.sh <X.Y.Z>   (depuis la racine du dépôt)
set -euo pipefail

readonly VERSION_PATTERN='^[0-9]+\.[0-9]+\.[0-9]+$'
readonly CRATE_PREFIX="hubergamof"

if (($# != 1)) || [[ ! "$1" =~ $VERSION_PATTERN ]]; then
	echo "Usage : $(basename "$0") <X.Y.Z>" >&2
	exit 2
fi
version="$1"

# Cargo.toml : seule la ligne `version` de [workspace.package].
awk -v version="$version" '
	/^\[/ { in_package = ($0 == "[workspace.package]") }
	in_package && /^version = / { print "version = \"" version "\""; next }
	{ print }
' Cargo.toml >Cargo.toml.new
mv Cargo.toml.new Cargo.toml

# Cargo.lock : la ligne `version` qui suit chaque crate du workspace.
awk -v version="$version" -v prefix="$CRATE_PREFIX" '
	/^name = / { workspace_crate = ($0 ~ "^name = \"" prefix "(-[a-z-]+)?\"$") }
	workspace_crate && /^version = / { print "version = \"" version "\""; workspace_crate = 0; next }
	{ print }
' Cargo.lock >Cargo.lock.new
mv Cargo.lock.new Cargo.lock

# Manifeste de l'extension Teams (embarquée dans le binaire) : elle annonce au
# gestionnaire la version qu'elle porte, qui doit être la sienne.
readonly EXTENSION_MANIFEST="ui/teams-extension/manifest.json"
if [[ ! -f "$EXTENSION_MANIFEST" ]]; then
	echo "$EXTENSION_MANIFEST introuvable (le script se lance depuis la racine du dépôt)." >&2
	exit 1
fi
sed -E 's/^(\t"version": ")[^"]*(",)$/\1'"$version"'\2/' "$EXTENSION_MANIFEST" >"$EXTENSION_MANIFEST.new"
if ! grep -q "^	\"version\": \"$version\",\$" "$EXTENSION_MANIFEST.new"; then
	rm -f "$EXTENSION_MANIFEST.new"
	echo "Ligne « version » introuvable dans $EXTENSION_MANIFEST." >&2
	exit 1
fi
mv "$EXTENSION_MANIFEST.new" "$EXTENSION_MANIFEST"
echo "Version du workspace et de l'extension fixée à $version."
