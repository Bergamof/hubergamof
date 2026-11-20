#!/usr/bin/env bash
# Installe le paquet dans un conteneur propre et vérifie ce qu'un utilisateur
# attend : dépendances résolues (dont pactl), fichiers posés, binaire lançable à
# la bonne version, unité et règle udev valides, scripts de maintenance sans
# erreur (sans systemd en cours d'exécution), lanceur de bureau valide et ses
# icônes, désinstallation propre.
#
# À lancer en root dans le conteneur (make package-smoke) :
#   smoke-test.sh <deb|arch> <version> <dossier des paquets>
set -euo pipefail

if (($# != 3)) || [[ "$1" != deb && "$1" != arch ]]; then
	echo "Usage : $(basename "$0") <deb|arch> <version> <dossier des paquets>" >&2
	exit 2
fi
kind="$1"
version="$2"
dist="$3"

readonly BINARY=/usr/bin/hubergamof
readonly HELPER=/usr/lib/hubergamof/hubergamof-update-helper
readonly UPDATE_SOCKET_UNIT=/usr/lib/systemd/system/hubergamof-update.socket
readonly UPDATE_SERVICE_UNIT=/usr/lib/systemd/system/hubergamof-update@.service
readonly UNIT=/usr/lib/systemd/user/hubergamof.service
readonly UDEV_RULE=/usr/lib/udev/rules.d/70-hubergamof.rules
# shellcheck source=packaging/desktop/install-desktop.sh
source "$(dirname "$(dirname "$(realpath "${BASH_SOURCE[0]}")")")/desktop/install-desktop.sh"
readonly DESKTOP_ENTRY="/usr/share/$DESKTOP_ENTRY_TARGET"
# Lanceur et icônes, tels que stage.sh les pose.
mapfile -t DESKTOP_FILES < <(desktop_entry_files | sed 's|^|/usr/share/|')
readonly DESKTOP_FILES
readonly EXPECTED_DESKTOP_EXEC='Exec="/usr/bin/hubergamof" open'
readonly EXPECTED_EXEC_START='ExecStart=/usr/bin/hubergamof --config-dir "%E/hubergamof"'
readonly PACTL_MIN_MAJOR=16

step() { printf '\n==> %s\n' "$*"; }
fail() {
	printf 'ÉCHEC : %s\n' "$*" >&2
	exit 1
}

install_package() {
	if [[ "$kind" == deb ]]; then
		export DEBIAN_FRONTEND=noninteractive
		apt-get update -qq
		apt-get install -y -qq "$dist/hubergamof_${version}_amd64.deb"
	else
		pacman -Syu --noconfirm
		pacman -U --noconfirm "$dist/hubergamof-${version}-1-x86_64.pkg.tar.zst"
	fi
}

remove_package() {
	if [[ "$kind" == deb ]]; then
		apt-get purge -y -qq hubergamof
	else
		pacman -R --noconfirm hubergamof
	fi
}

check_files() {
	local path
	for path in "$BINARY" "$HELPER" "$UPDATE_SOCKET_UNIT" "$UPDATE_SERVICE_UNIT" "$UNIT" "$UDEV_RULE" "${DESKTOP_FILES[@]}"; do
		[[ -e "$path" ]] || fail "fichier absent : $path"
	done
	[[ "$(stat -c %a "$BINARY")" == 755 ]] || fail "$BINARY n'est pas en 0755"
	[[ "$(stat -c %a "$HELPER")" == 755 && "$(stat -c %U "$HELPER")" == root ]] || fail "$HELPER doit être en 0755 et à root"
	grep -qF -- 'ListenStream=/run/hubergamof-update.sock' "$UPDATE_SOCKET_UNIT" || fail "socket des mises à jour inattendu"
	grep -qF -- "ExecStart=$HELPER" "$UPDATE_SERVICE_UNIT" || fail "service des mises à jour sans l'assistant"
	grep -qF -- "$EXPECTED_EXEC_START" "$UNIT" || fail "ExecStart inattendu dans $UNIT"
	grep -q 'TAG+="uaccess"' "$UDEV_RULE" || fail "règle udev sans uaccess"
	# Hors commentaires : l'en-tête de la règle cite MODE="0666" pour expliquer pourquoi on l'évite.
	if grep -v '^[[:space:]]*#' "$UDEV_RULE" | grep -q 'MODE='; then fail "règle udev ouverte à tous"; fi
	# Récepteur Logitech : uniquement le Logi Bolt (c548), jamais le Unifying (c52b, la saisie
	# passe par son HID++), jamais un nœud enfant, et jamais un uaccess sans filtre de produit.
	local active
	active="$(grep -v '^[[:space:]]*#' "$UDEV_RULE")"
	if grep -q 'c52b' <<<"$active"; then fail "règle udev ouverte au récepteur Unifying"; fi
	grep -qF -- 'DEVPATH!="*/0003:*/0003:*/hidraw/*"' <<<"$active" || fail "règle udev sans exclusion des nœuds enfants"
	if grep 'idVendor}=="046d"' <<<"$active" | grep -v 'idProduct}=="c548"' | grep -q 'uaccess'; then fail "uaccess Logitech sans filtre de produit"; fi
	# /dev/uinput : un nœud précis, ouvert par uaccess (jamais par un mode), sans autre nœud nommé.
	grep -qF -- 'KERNEL=="uinput"' <<<"$active" || fail "règle udev sans /dev/uinput"
	if grep -F 'KERNEL==' <<<"$active" | grep -vF 'KERNEL=="uinput"' | grep -q .; then fail "règle udev par nom de nœud autre que uinput"; fi
	if grep -F 'KERNEL=="uinput"' <<<"$active" | grep -qv 'TAG+="uaccess"'; then fail "uinput sans uaccess"; fi
}

check_binary() {
	local reported
	reported="$("$BINARY" --version)"
	[[ "$reported" == "hubergamof $version" ]] || fail "version rapportée « $reported », attendue « hubergamof $version »"
}

# Dépendance obligatoire du module Audio : pactl d'une version assez récente.
# Les mises à jour automatiques : l'assistant, lancé comme systemd le fait (la
# connexion est son entrée), refuse sans rien installer une connexion vide et
# répond sur une ligne.
check_update_helper() {
	local output
	if output="$("$HELPER" </dev/null 2>/dev/null)"; then fail "l'assistant a accepté une connexion vide"; fi
	[[ "$output" == error\ * ]] || fail "réponse inattendue de l'assistant : $output"
}

check_pactl() {
	local output major
	command -v pactl >/dev/null || fail "pactl absent : dépendance audio non installée"
	output="$(pactl --version)"
	major="$(sed -nE 's/^pactl ([0-9]+).*/\1/p' <<<"$output")"
	((major >= PACTL_MIN_MAJOR)) || fail "pactl trop ancien : $output"
}

check_systemd_files() {
	if command -v systemd-analyze >/dev/null; then
		XDG_RUNTIME_DIR=/tmp systemd-analyze --user verify "$UNIT" || fail "unité systemd invalide"
		systemd-analyze verify "$UPDATE_SOCKET_UNIT" "$UPDATE_SERVICE_UNIT" || fail "unités des mises à jour invalides"
	else
		echo "systemd-analyze absent : unité non validée."
	fi
	if command -v udevadm >/dev/null && udevadm verify --help >/dev/null 2>&1; then
		udevadm verify "$UDEV_RULE" || fail "règle udev invalide"
	else
		echo "udevadm verify indisponible : règle non validée."
	fi
}

# Le lanceur relance le service par `hubergamof open` ; son format est vérifié
# par desktop-file-validate (installé ici, absent des images de base).
check_desktop_entry() {
	grep -qxF -- "$EXPECTED_DESKTOP_EXEC" "$DESKTOP_ENTRY" || fail "Exec inattendu dans $DESKTOP_ENTRY"
	if [[ "$kind" == deb ]]; then
		apt-get install -y -qq desktop-file-utils
	else
		pacman -S --noconfirm desktop-file-utils
	fi
	desktop-file-validate "$DESKTOP_ENTRY" || fail "lanceur de bureau invalide"
	"$BINARY" open --help >/dev/null || fail "sous-commande « open » absente"
}

check_arch_lint() {
	local report
	pacman -S --noconfirm namcap
	report="$(namcap "$dist/hubergamof-${version}-1-x86_64.pkg.tar.zst" || true)"
	printf '%s\n' "$report"
	if grep -q ' E: ' <<<"$report"; then fail "namcap signale des erreurs"; fi
}

check_removed() {
	local path
	for path in "$BINARY" "$HELPER" "$UPDATE_SOCKET_UNIT" "$UPDATE_SERVICE_UNIT" "$UNIT" "$UDEV_RULE" "${DESKTOP_FILES[@]}"; do
		[[ ! -e "$path" ]] || fail "fichier resté après désinstallation : $path"
	done
}

step "Installation ($kind $version)"
install_package
step "Fichiers"
check_files
step "Binaire"
check_binary
step "Assistant des mises à jour"
check_update_helper
step "pactl (module Audio)"
check_pactl
step "Unité systemd et règle udev"
check_systemd_files
step "Lanceur de bureau"
check_desktop_entry
if [[ "$kind" == arch ]]; then
	step "namcap"
	check_arch_lint
fi
step "Désinstallation"
remove_package
check_removed
echo
echo "Paquet $kind $version : tous les contrôles passent."
