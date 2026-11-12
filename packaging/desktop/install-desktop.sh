# shellcheck shell=bash
# HuBergamof — pose du lanceur de bureau et de ses icônes, commune à
# packaging/install.sh (~/.local/share), packaging/uninstall.sh et
# packaging/package/stage.sh (/usr/share). Ce fichier est sourcé, jamais exécuté
# directement.

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Icônes de l'application : celles du tray (source SVG + pixmaps).
ICON_SOURCE_DIR="$(dirname "$(dirname "$DESKTOP_DIR")")/crates/manager/assets/tray"
readonly DESKTOP_DIR ICON_SOURCE_DIR
readonly DESKTOP_ENTRY_NAME="hubergamof.desktop"
readonly DESKTOP_ENTRY_TARGET="applications/$DESKTOP_ENTRY_NAME"
readonly ICON_NAME="hubergamof"
# Variantes d'icône : vectorielle, puis pixmaps (côté en pixels) du tray.
readonly ICON_VARIANTS=(scalable 22 32 48 64)

# Échappe un chemin pour la clé Exec d'un lanceur : entre guillemets, `"`, `` ` ``,
# `$` et `\` préfixés d'un antislash, puis chaque antislash doublé (échappement
# des chaînes du format), `%` doublé (codes de champ).
escape_for_desktop_exec() {
	local value="$1"
	value="${value//\\/\\\\}"
	value="${value//\"/\\\"}"
	value="${value//\`/\\\`}"
	value="${value//\$/\\\$}"
	value="${value//\\/\\\\}"
	value="${value//%/%%}"
	printf '"%s"' "$value"
}

icon_source() { # icon_source <variante>
	if [[ "$1" == scalable ]]; then
		printf '%s\n' "$ICON_SOURCE_DIR/$ICON_NAME.svg"
	else
		printf '%s\n' "$ICON_SOURCE_DIR/$ICON_NAME-$1.png"
	fi
}

icon_target() { # icon_target <variante>, relatif au dossier share
	if [[ "$1" == scalable ]]; then
		printf '%s\n' "icons/hicolor/scalable/apps/$ICON_NAME.svg"
	else
		printf '%s\n' "icons/hicolor/${1}x${1}/apps/$ICON_NAME.png"
	fi
}

# install_desktop_entry <binaire installé> <dossier share>
# Pose <share>/applications/hubergamof.desktop et les icônes hicolor.
install_desktop_entry() {
	local binary="$1" share="$2" template entry variant
	template="$(<"$DESKTOP_DIR/$DESKTOP_ENTRY_NAME")"
	entry="${template//@BINARY@/"$(escape_for_desktop_exec "$binary")"}"
	install -d -m 0755 "$share/applications"
	printf '%s\n' "$entry" >"$share/$DESKTOP_ENTRY_TARGET"
	chmod 0644 "$share/$DESKTOP_ENTRY_TARGET"
	for variant in "${ICON_VARIANTS[@]}"; do
		install -D -m 0644 "$(icon_source "$variant")" "$share/$(icon_target "$variant")"
	done
}

# Fichiers posés par install_desktop_entry, relatifs au dossier share.
desktop_entry_files() {
	local variant
	printf '%s\n' "$DESKTOP_ENTRY_TARGET"
	for variant in "${ICON_VARIANTS[@]}"; do
		icon_target "$variant"
	done
}
