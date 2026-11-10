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
echo "Version du workspace fixée à $version."
