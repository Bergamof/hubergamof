# shellcheck shell=bash disable=SC2034  # variables consommées par les scripts qui sourcent ce fichier
# HuBergamof — constantes et fonctions partagées par install.sh et uninstall.sh.
# Ce fichier est sourcé, jamais exécuté directement.

PACKAGING_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$PACKAGING_DIR")"
readonly PACKAGING_DIR REPO_ROOT

readonly SERVICE_NAME="hubergamof.service"
readonly BIN_DIR="$HOME/.local/bin"
readonly INSTALLED_BINARY="$BIN_DIR/hubergamof"
readonly UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
readonly INSTALLED_UNIT="$UNIT_DIR/$SERVICE_NAME"
# Lanceur de bureau et icônes (menus et bureaux suivent ce dossier d'eux-mêmes).
readonly DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
readonly UDEV_RULE_NAME="70-hubergamof.rules"
readonly UDEV_RULE_SOURCE="$PACKAGING_DIR/udev/$UDEV_RULE_NAME"
readonly UDEV_RULE_TARGET="/etc/udev/rules.d/$UDEV_RULE_NAME"

# Options communes, remplies par parse_common_options.
SKIP_UDEV=false
ASSUME_YES=false

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mAttention :\033[0m %s\n' "$*" >&2; }
die() {
	printf '\033[1;31mErreur :\033[0m %s\n' "$*" >&2
	exit 1
}

# usage_text est fourni par le script appelant.
parse_common_options() {
	while (($# > 0)); do
		case "$1" in
		--no-udev) SKIP_UDEV=true ;;
		-y | --yes) ASSUME_YES=true ;;
		-h | --help)
			usage_text
			exit 0
			;;
		*)
			usage_text >&2
			exit 2
			;;
		esac
		shift
	done
}

refuse_root() {
	if ((EUID == 0)); then
		die "à lancer en tant qu'utilisateur (le service est un service utilisateur) ; sudo est demandé seulement pour la règle udev."
	fi
}

require_user_systemd() {
	command -v systemctl >/dev/null || die "systemctl introuvable : systemd est requis."
	systemctl --user show-environment >/dev/null 2>&1 \
		|| die "gestionnaire systemd utilisateur injoignable (lancer depuis la session graphique)."
}

# Demande une confirmation ; refuse sans terminal interactif sauf avec --yes.
confirm() {
	local question="$1" answer
	if [[ "$ASSUME_YES" == true ]]; then
		return 0
	fi
	if [[ ! -t 0 ]]; then
		warn "pas de terminal interactif : étape ignorée (relancer avec --yes pour l'accepter)."
		return 1
	fi
	read -r -p "$question [o/N] " answer
	[[ "$answer" =~ ^[oOyY]$ ]]
}

# Version majeure minimale de pactl (--format=json, utilisé par le module Audio).
readonly PACTL_MIN_MAJOR=16

# Avertit (sans bloquer) si le module Audio ne pourra pas fonctionner : pactl
# absent (sur Ubuntu, il est dans pulseaudio-utils) ou trop ancien.
check_pactl() {
	local version major
	if ! command -v pactl >/dev/null; then
		warn "pactl introuvable : le module Audio sera indisponible. Ubuntu/Debian : sudo apt install pulseaudio-utils ; Arch : sudo pacman -S libpulse."
		return
	fi
	# `|| version=` : un pactl cassé ne doit pas arrêter l'installation (set -e, pipefail).
	version="$(LC_ALL=C pactl --version 2>/dev/null | head -n 1)" || version=""
	major="${version##* }"
	major="${major%%.*}"
	if [[ ! "$major" =~ ^[0-9]+$ ]] || ((10#$major < PACTL_MIN_MAJOR)); then
		warn "pactl ${PACTL_MIN_MAJOR} ou plus est requis par le module Audio (trouvé : « ${version:-version illisible} »)."
	fi
}

reload_udev_rules() {
	sudo udevadm control --reload-rules
	sudo udevadm trigger --action=change --subsystem-match=usb --attr-match=idVendor=0fd9
	sudo udevadm trigger --action=change --subsystem-match=hidraw
}
