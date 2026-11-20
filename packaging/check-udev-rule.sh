#!/usr/bin/env bash
# Vérifie les invariants de sécurité de la règle udev (voir docs/security.md) par liste
# blanche : chaque ligne active doit être exactement l'une des règles attendues. Une
# règle ajoutée (accès par mode, groupe ou propriétaire, autre nœud, autre produit
# Logitech, RUN, PROGRAM…) ou modifiée est donc refusée, et ce script change avec la règle.
# L'accès passe par `uaccess` (utilisateur de la session active) ; côté Logitech, seul le
# Logi Bolt est ouvert, par son interface HID++ et sans nœud enfant ; /dev/uinput est le
# seul nœud désigné par son nom.
#
# Usage : check-udev-rule.sh <fichier de règles>
# Code 0 : conforme ; 1 : une ligne n'est pas attendue (laquelle, sur stderr).
set -euo pipefail

if (($# != 1)); then
	echo "Usage : $(basename "$0") <fichier de règles>" >&2
	exit 2
fi
rule_file="$1"

readonly -a EXPECTED=(
	'SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{idVendor}=="0fd9", TAG+="uaccess"'
	'SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0fd9", TAG+="uaccess"'
	'SUBSYSTEM=="hidraw", SUBSYSTEMS=="usb", ATTRS{bInterfaceNumber}=="02", ENV{HUBERGAMOF_HIDPP}="1"'
	'SUBSYSTEM=="hidraw", ENV{HUBERGAMOF_HIDPP}=="1", DEVPATH!="*/0003:*/0003:*/hidraw/*", ATTRS{idVendor}=="046d", ATTRS{idProduct}=="c548", TAG+="uaccess"'
	'KERNEL=="uinput", SUBSYSTEM=="misc", OPTIONS+="static_node=uinput", TAG+="uaccess"'
)

fail() {
	echo "règle udev non conforme : $*" >&2
	exit 1
}

seen=0
while IFS= read -r line; do
	# Ni commentaires ni lignes vides (l'en-tête cite MODE="0666" pour expliquer pourquoi
	# on l'évite).
	[[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
	known=0
	for expected in "${EXPECTED[@]}"; do
		[[ "$line" == "$expected" ]] && known=1
	done
	((known == 1)) || fail "ligne inattendue : $line"
	seen=$((seen + 1))
done <"$rule_file"
((seen == ${#EXPECTED[@]})) || fail "$seen règle(s) active(s) au lieu de ${#EXPECTED[@]}"
