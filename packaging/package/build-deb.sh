#!/usr/bin/env bash
# Construit le paquet Debian/Ubuntu (dpkg-deb, sans outil tiers).
#
# Dépendances obligatoires : bibliothèques du binaire (calculées par
# dpkg-shlibdeps), udev, systemd, et le nécessaire au module Audio : pactl
# (pulseaudio-utils ≥ 16) et un serveur de son (PipeWire ou PulseAudio).
# Optionnels : Docker, portail de fichiers, extension de tray GNOME.
#
# Usage : build-deb.sh <binaire> <version> <dossier de sortie>
set -euo pipefail

if (($# != 3)); then
	echo "Usage : $(basename "$0") <binaire> <version> <dossier de sortie>" >&2
	exit 2
fi
binary="$(realpath "$1")"
version="$2"
out_dir="$3"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PACKAGE="hubergamof"
readonly MAINTAINER="HuBergamof <noreply@users.noreply.github.com>"
readonly HOMEPAGE="https://github.com/Bergamof/hubergamof"
readonly RUNTIME_DEPENDS="udev, systemd, pulseaudio-utils (>= 1:16), pipewire-pulse | pulseaudio, xdg-utils"
readonly RECOMMENDS="xdg-desktop-portal"
readonly SUGGESTS="docker.io | docker-ce, gnome-shell-extension-appindicator"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
root="$work/root"
control_dir="$root/DEBIAN"

"$here/stage.sh" "$binary" "$root"

# Bibliothèques partagées du binaire : dpkg-shlibdeps lit un debian/control.
shlibs_dir="$work/shlibs"
mkdir -p "$shlibs_dir/debian"
printf 'Source: %s\n\nPackage: %s\nArchitecture: amd64\n' "$PACKAGE" "$PACKAGE" >"$shlibs_dir/debian/control"
shlibs="$(cd "$shlibs_dir" && dpkg-shlibdeps -O -e"$binary" | sed -n 's/^shlibs:Depends=//p')"
[[ -n "$shlibs" ]] || { echo "dpkg-shlibdeps n'a produit aucune dépendance." >&2; exit 1; }

doc_dir="$root/usr/share/doc/$PACKAGE"
install -d -m 0755 "$doc_dir"
cat >"$doc_dir/copyright" <<COPYRIGHT
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: $PACKAGE
Source: $HOMEPAGE

Files: *
Copyright: HuBergamof contributors
License: GPL-3+

License: GPL-3+
 This program is free software: you can redistribute it and/or modify it under
 the terms of the GNU General Public License as published by the Free Software
 Foundation, either version 3 of the License, or (at your option) any later
 version.
 .
 On Debian systems, the full text of the GNU General Public License version 3
 can be found in /usr/share/common-licenses/GPL-3.
COPYRIGHT
{
	printf '%s (%s) unstable; urgency=medium\n\n' "$PACKAGE" "$version"
	printf '  * Release %s, notes: %s/releases/tag/v%s\n\n' "$version" "$HOMEPAGE" "$version"
	printf ' -- %s  %s\n' "$MAINTAINER" "$(date -R -u)"
} | gzip -9n >"$doc_dir/changelog.gz"
chmod 0644 "$doc_dir/copyright" "$doc_dir/changelog.gz"

installed_size="$(du -sk --exclude=DEBIAN "$root" | cut -f1)"
install -d -m 0755 "$control_dir"
cat >"$control_dir/control" <<CONTROL
Package: $PACKAGE
Version: $version
Architecture: amd64
Maintainer: $MAINTAINER
Installed-Size: $installed_size
Depends: $shlibs, $RUNTIME_DEPENDS
Recommends: $RECOMMENDS
Suggests: $SUGGESTS
Section: utils
Priority: optional
Homepage: $HOMEPAGE
Description: Stream Deck manager with a local web interface
 HuBergamof drives Elgato Stream Deck devices (Stream Deck+ first) with
 modules (audio, Docker, IntelliJ, Claude, pomodoro, clock, notifications)
 and a web configuration interface served on 127.0.0.1 only.
 .
 The service runs as a systemd user unit; the udev rule gives the logged-in
 user access to the devices.
CONTROL

# Scripts de maintenance : hooks.sh + l'appel correspondant à l'action dpkg.
write_script() { # write_script <nom> <corps>
	{
		printf '#!/bin/sh\n# Généré par packaging/package/build-deb.sh\n'
		cat "$here/hooks.sh"
		printf '\n%s\n' "$2"
	} >"$control_dir/$1"
	chmod 0755 "$control_dir/$1"
}
# shellcheck disable=SC2016  # $1 est évalué par le script généré, pas ici
write_script postinst 'case "$1" in
configure) if [ -n "$2" ]; then hubergamof_post_install upgrade; else hubergamof_post_install install; fi ;;
esac
exit 0'
# shellcheck disable=SC2016
write_script prerm 'case "$1" in
remove | deconfigure) hubergamof_pre_remove ;;
esac
exit 0'
# shellcheck disable=SC2016
write_script postrm 'case "$1" in
remove | purge) hubergamof_post_remove ;;
esac
exit 0'

(cd "$root" && find . -path ./DEBIAN -prune -o -type f -print0 | sort -z | xargs -0 md5sum | sed 's|  \./|  |' >"$control_dir/md5sums")
chmod 0644 "$control_dir/control" "$control_dir/md5sums"

mkdir -p "$out_dir"
deb="$out_dir/${PACKAGE}_${version}_amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$root" "$deb" >/dev/null
echo "Paquet créé : $deb"
