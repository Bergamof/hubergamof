#!/usr/bin/env bash
# HuBergamof — désinstallation pour l'utilisateur courant (idempotente).
# Retire le service utilisateur, le binaire, le lanceur de bureau et, après
# confirmation, la règle udev.
# La configuration (dossier config/ du dépôt) est conservée.
set -euo pipefail

# shellcheck source=packaging/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
# shellcheck source=packaging/desktop/install-desktop.sh
source "$PACKAGING_DIR/desktop/install-desktop.sh"

usage_text() {
	cat <<EOF
Usage : $(basename "$0") [--no-udev] [--yes]

Désinstalle HuBergamof pour l'utilisateur courant (la configuration est conservée).
  --no-udev  ne pas retirer la règle udev (aucun appel à sudo)
  -y, --yes  accepter la suppression de la règle udev sans la demander
  -h, --help afficher cette aide
EOF
}

remove_unit() {
	if [[ -f "$INSTALLED_UNIT" ]]; then
		info "Arrêt et suppression du service utilisateur"
		systemctl --user disable --now "$SERVICE_NAME" || warn "arrêt du service impossible."
		rm -f "$INSTALLED_UNIT"
		systemctl --user daemon-reload
		systemctl --user reset-failed "$SERVICE_NAME" 2>/dev/null || true
	else
		info "Service utilisateur absent."
	fi
}

remove_binary() {
	if [[ -e "$INSTALLED_BINARY" ]]; then
		info "Suppression de $INSTALLED_BINARY"
		rm -f "$INSTALLED_BINARY"
	else
		info "Binaire absent."
	fi
}

remove_launcher() {
	local file
	info "Suppression du lanceur de bureau et des icônes"
	while IFS= read -r file; do
		rm -f "$DATA_DIR/$file"
	done < <(desktop_entry_files)
}

remove_udev_rule() {
	if [[ "$SKIP_UDEV" == true ]]; then
		info "Règle udev conservée (--no-udev)."
		return
	fi
	if [[ ! -e "$UDEV_RULE_TARGET" ]]; then
		info "Règle udev absente."
		return
	fi
	printf '\nLa règle %s sera supprimée avec sudo, puis udev rechargé.\n\n' "$UDEV_RULE_TARGET"
	if ! confirm "Supprimer la règle udev ?"; then
		warn "règle udev conservée."
		return
	fi
	sudo rm -f "$UDEV_RULE_TARGET"
	reload_udev_rules
	info "Règle udev supprimée."
}

main() {
	parse_common_options "$@"
	refuse_root
	require_user_systemd
	remove_unit
	remove_binary
	remove_launcher
	remove_udev_rule
	info "HuBergamof désinstallé (configuration conservée dans $REPO_ROOT/config)."
}

main "$@"
