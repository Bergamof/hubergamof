#!/usr/bin/env bash
# Dispose les fichiers communs aux paquets .deb et Arch dans une arborescence
# racine : binaire, règle udev, unité systemd utilisateur, lanceur de bureau et
# icônes (les caches d'icônes et de lanceurs sont rafraîchis par les déclencheurs
# de dpkg et les hooks de pacman).
#
# Le service lit sa configuration dans ~/.config/hubergamof (%E = XDG_CONFIG_HOME
# ou ~/.config, spécificateur systemd).
#
# Usage : stage.sh <binaire> <dossier racine>
set -euo pipefail

if (($# != 2)); then
	echo "Usage : $(basename "$0") <binaire> <dossier racine>" >&2
	exit 2
fi
binary="$1"
root="$2"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
packaging="$(dirname "$here")"
# shellcheck source=packaging/desktop/install-desktop.sh
source "$packaging/desktop/install-desktop.sh"

readonly INSTALLED_BINARY="/usr/bin/hubergamof"
readonly CONFIG_DIR_SPECIFIER='%E/hubergamof'

[[ -x "$binary" ]] || { echo "Binaire introuvable : $binary (lancer « make build »)." >&2; exit 1; }

install -D -m 0755 "$binary" "$root$INSTALLED_BINARY"
install -D -m 0644 "$packaging/udev/70-hubergamof.rules" "$root/usr/lib/udev/rules.d/70-hubergamof.rules"

template="$(<"$packaging/systemd/hubergamof.service")"
unit="${template//@BINARY@/$INSTALLED_BINARY}"
unit="${unit//@CONFIG_DIR@/$CONFIG_DIR_SPECIFIER}"
install -d -m 0755 "$root/usr/lib/systemd/user"
printf '%s\n' "$unit" >"$root/usr/lib/systemd/user/hubergamof.service"
chmod 0644 "$root/usr/lib/systemd/user/hubergamof.service"

install_desktop_entry "$INSTALLED_BINARY" "$root/usr/share"
