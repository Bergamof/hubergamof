# shellcheck shell=bash
# HuBergamof — pose du lanceur de bureau et de ses icônes, commune à
# packaging/install.sh (~/.local/share) et packaging/package/stage.sh (/usr/share).
# Ce fichier est sourcé, jamais exécuté directement.

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Icônes de l'application : celles du tray (source SVG + pixmaps).
ICON_SOURCE_DIR="$(dirname "$(dirname "$DESKTOP_DIR")")/crates/manager/assets/tray"
readonly DESKTOP_DIR ICON_SOURCE_DIR
readonly DESKTOP_ENTRY_NAME="hubergamof.desktop"
readonly ICON_NAME="hubergamof"
readonly ICON_PIXEL_SIZES=(22 32 48 64)

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

# install_desktop_entry <binaire installé> <dossier share>
# Pose <share>/applications/hubergamof.desktop et les icônes hicolor.
install_desktop_entry() {
	local binary="$1" share="$2" template entry size
	template="$(<"$DESKTOP_DIR/$DESKTOP_ENTRY_NAME")"
	entry="${template//@BINARY@/"$(escape_for_desktop_exec "$binary")"}"
	install -d -m 0755 "$share/applications"
	printf '%s\n' "$entry" >"$share/applications/$DESKTOP_ENTRY_NAME"
	chmod 0644 "$share/applications/$DESKTOP_ENTRY_NAME"
	install -D -m 0644 "$ICON_SOURCE_DIR/$ICON_NAME.svg" "$share/icons/hicolor/scalable/apps/$ICON_NAME.svg"
	for size in "${ICON_PIXEL_SIZES[@]}"; do
		install -D -m 0644 "$ICON_SOURCE_DIR/$ICON_NAME-$size.png" \
			"$share/icons/hicolor/${size}x${size}/apps/$ICON_NAME.png"
	done
}

# Fichiers posés par install_desktop_entry, relatifs au dossier share.
desktop_entry_files() {
	local size
	printf '%s\n' "applications/$DESKTOP_ENTRY_NAME" "icons/hicolor/scalable/apps/$ICON_NAME.svg"
	for size in "${ICON_PIXEL_SIZES[@]}"; do
		printf '%s\n' "icons/hicolor/${size}x${size}/apps/$ICON_NAME.png"
	done
}
