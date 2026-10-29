#!/usr/bin/env bash
# Bergadeck — installation pour l'utilisateur courant (idempotente) :
#   1. binaire release dans ~/.local/bin ;
#   2. service systemd utilisateur, activé et (re)démarré ;
#   3. règle udev pour accéder aux Stream Deck sans root (sudo, après confirmation).
# Prérequis : `make build` (le binaire release embarque l'UI).
set -euo pipefail

# shellcheck source=packaging/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

readonly RELEASE_BINARY="$REPO_ROOT/target/release/bergadeck"
readonly UNIT_TEMPLATE="$PACKAGING_DIR/systemd/$SERVICE_NAME"
# La configuration reste pour l'instant dans le dépôt (déplacement vers ~/.config prévu).
readonly CONFIG_DIR="$REPO_ROOT/config"
readonly CONFIG_PLACEHOLDER="@CONFIG_DIR@"

usage_text() {
	cat <<EOF
Usage : $(basename "$0") [--no-udev] [--yes]

Installe Bergadeck pour l'utilisateur courant.
  --no-udev  ne pas installer la règle udev (aucun appel à sudo)
  -y, --yes  accepter l'installation de la règle udev sans la demander
  -h, --help afficher cette aide
EOF
}

# Échappe une valeur pour un argument entre guillemets d'une ligne ExecStart :
# antislash et guillemet sont échappés, % (spécificateur) et $ (variable) doublés.
escape_for_exec_start() {
	local value="$1"
	value="${value//\\/\\\\}"
	value="${value//\"/\\\"}"
	value="${value//%/%%}"
	value="${value//\$/\$\$}"
	printf '%s' "$value"
}

install_binary() {
	[[ -x "$RELEASE_BINARY" ]] || die "binaire introuvable ($RELEASE_BINARY) : lancer d'abord « make build »."
	info "Installation du binaire dans $INSTALLED_BINARY"
	install -D -m 0755 "$RELEASE_BINARY" "$INSTALLED_BINARY"
	if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
		warn "$BIN_DIR n'est pas dans le PATH (sans incidence pour le service)."
	fi
}

install_unit() {
	local template unit
	template="$(<"$UNIT_TEMPLATE")"
	unit="${template//"$CONFIG_PLACEHOLDER"/"$(escape_for_exec_start "$CONFIG_DIR")"}"
	info "Installation du service utilisateur dans $INSTALLED_UNIT"
	mkdir -p "$UNIT_DIR" "$CONFIG_DIR"
	printf '%s\n' "$unit" >"$INSTALLED_UNIT"
	systemctl --user daemon-reload
	# reenable : recrée les liens d'après [Install] (retire ceux d'une version
	# antérieure de l'unité, ex. default.target).
	systemctl --user reenable "$SERVICE_NAME"
	# restart plutôt que start : une réinstallation prend en compte le nouveau binaire.
	systemctl --user restart "$SERVICE_NAME"
}

install_udev_rule() {
	if [[ "$SKIP_UDEV" == true ]]; then
		info "Règle udev ignorée (--no-udev)."
		return
	fi
	if cmp -s "$UDEV_RULE_SOURCE" "$UDEV_RULE_TARGET"; then
		info "Règle udev déjà à jour ($UDEV_RULE_TARGET)."
		return
	fi
	cat <<EOF

Règle udev : pour piloter les Stream Deck sans être root, la règle
  $UDEV_RULE_SOURCE
doit être copiée dans $UDEV_RULE_TARGET, puis udev rechargé.
Elle accorde l'accès aux appareils Elgato (vendor 0fd9) à l'utilisateur de la
session locale active uniquement (TAG+="uaccess"). Cela nécessite sudo.

EOF
	if ! confirm "Installer la règle udev avec sudo ?"; then
		warn "règle udev non installée : les appareils resteront inaccessibles sans root."
		return
	fi
	sudo install -D -m 0644 "$UDEV_RULE_SOURCE" "$UDEV_RULE_TARGET"
	reload_udev_rules
	info "Règle udev installée. Débrancher puis rebrancher l'appareil si besoin."
}

main() {
	parse_common_options "$@"
	refuse_root
	require_user_systemd
	install_binary
	install_unit
	install_udev_rule
	info "Bergadeck installé. État : systemctl --user status $SERVICE_NAME"
}

main "$@"
