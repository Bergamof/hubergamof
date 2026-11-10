# shellcheck shell=sh
# HuBergamof — scripts de maintenance communs aux paquets .deb et Arch.
#
# Fichier POSIX sh, jamais exécuté seul : build-deb.sh l'inclut dans
# postinst/prerm/postrm, build-arch.sh dans hubergamof.install. Il tourne en
# root et ne doit jamais faire échouer l'installation ou la désinstallation :
# chaque commande tolère l'échec, un message prévient quand une étape est ignorée.
#
# Le service est une unité systemd *utilisateur* (bus D-Bus, Wayland et audio de
# la session) : « activer » = l'activer pour tous les utilisateurs (--global,
# effectif au prochain login), « démarrer » = le démarrer dans les sessions
# graphiques déjà ouvertes.

HUBERGAMOF_UNIT="hubergamof.service"
# Premier uid d'un compte humain (UID_MIN de login.defs) : ignore root et les comptes système.
HUBERGAMOF_FIRST_USER_UID=1000
# Surchargeable pour les tests : présence de systemd en cours d'exécution.
HUBERGAMOF_SYSTEMD_DIR="${HUBERGAMOF_SYSTEMD_DIR:-/run/systemd/system}"

hubergamof_note() { printf 'hubergamof : %s\n' "$*" >&2; }

hubergamof_systemd_running() { [ -d "$HUBERGAMOF_SYSTEMD_DIR" ]; }

# Recharge les règles udev puis rejoue les évènements d'ajout : un Stream Deck déjà
# branché reçoit son accès sans débranchement.
hubergamof_reload_udev() {
	command -v udevadm >/dev/null 2>&1 || return 0
	hubergamof_systemd_running || return 0
	udevadm control --reload-rules >/dev/null 2>&1 || true
	udevadm trigger --action=add --subsystem-match=usb --subsystem-match=hidraw >/dev/null 2>&1 || true
}

# Applique `systemctl --user <action>` dans chaque session graphique ouverte.
# Usage : hubergamof_in_graphical_sessions <action…>
hubergamof_in_graphical_sessions() {
	command -v loginctl >/dev/null 2>&1 || return 0
	command -v systemctl >/dev/null 2>&1 || return 0
	hubergamof_systemd_running || return 0
	loginctl list-users --no-legend 2>/dev/null | while read -r uid name _; do
		[ "$uid" -ge "$HUBERGAMOF_FIRST_USER_UID" ] 2>/dev/null || continue
		manager="$name@.host"
		systemctl --user --machine="$manager" --quiet is-active graphical-session.target >/dev/null 2>&1 || continue
		systemctl --user --machine="$manager" "$@" >/dev/null 2>&1 \
			|| hubergamof_note "« systemctl --user $* » a échoué pour $name (le service démarrera à la prochaine connexion)."
	done
}

hubergamof_post_install() {
	hubergamof_reload_udev
	if hubergamof_systemd_running && command -v systemctl >/dev/null 2>&1; then
		systemctl --global enable "$HUBERGAMOF_UNIT" >/dev/null 2>&1 \
			|| hubergamof_note "activation globale du service impossible : « systemctl --user enable --now $HUBERGAMOF_UNIT »."
		hubergamof_in_graphical_sessions daemon-reload
		# restart : démarre le service, ou reprend le nouveau binaire après une mise à jour.
		hubergamof_in_graphical_sessions restart "$HUBERGAMOF_UNIT"
	fi
	hubergamof_note "service actif dans les sessions graphiques ouvertes, au prochain login sinon."
	hubergamof_note "module Docker : l'utilisateur doit appartenir au groupe « docker »."
}

hubergamof_pre_remove() {
	hubergamof_in_graphical_sessions stop "$HUBERGAMOF_UNIT"
	if hubergamof_systemd_running && command -v systemctl >/dev/null 2>&1; then
		systemctl --global disable "$HUBERGAMOF_UNIT" >/dev/null 2>&1 || true
	fi
}

hubergamof_post_remove() {
	hubergamof_in_graphical_sessions daemon-reload
	hubergamof_reload_udev
}
