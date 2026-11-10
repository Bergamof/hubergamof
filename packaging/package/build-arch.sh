#!/usr/bin/env bash
# Construit le paquet Arch Linux / Manjaro avec makepkg. À lancer en utilisateur
# non-root (makepkg le refuse) : `make package-arch` le fait dans un conteneur
# Arch, ce qui évite aussi d'avoir pacman sur la machine de build.
#
# Usage : build-arch.sh <binaire> <version> <dossier de sortie>
set -euo pipefail

if (($# != 3)); then
	echo "Usage : $(basename "$0") <binaire> <version> <dossier de sortie>" >&2
	exit 2
fi
binary="$(realpath "$1")"
version="$2"
out_dir="$(realpath -m "$3")"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(dirname "$(dirname "$here")")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

"$here/stage.sh" "$binary" "$work/stage"
cp "$here/PKGBUILD" "$repo_root/LICENSE" "$work/"
# hooks.sh en POSIX sh + les fonctions que pacman appelle (le fichier est sourcé par bash).
{
	cat "$here/hooks.sh"
	cat <<'FUNCTIONS'

post_install() { hubergamof_post_install; }
post_upgrade() { hubergamof_post_install; }
pre_remove() { hubergamof_pre_remove; }
post_remove() { hubergamof_post_remove; }
FUNCTIONS
} >"$work/hubergamof.install"

mkdir -p "$out_dir"
(cd "$work" && HUBERGAMOF_VERSION="$version" PKGDEST="$out_dir" makepkg --nodeps --force --nocolor)
ls "$out_dir"/hubergamof-"$version"-*.pkg.tar.zst
