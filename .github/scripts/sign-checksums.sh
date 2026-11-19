#!/usr/bin/env bash
# Signe SHA256SUMS (Ed25519) : produit SHA256SUMS.sig à côté, la signature que
# l'assistant de mise à jour vérifie avec la clé publique embarquée dans le
# binaire avant d'installer quoi que ce soit.
#
# La clé privée (PEM) vient de la variable d'environnement RELEASE_SIGNING_KEY
# (secret du dépôt) : jamais d'un argument, jamais écrite ailleurs que dans un
# dossier temporaire privé supprimé en sortie. La signature est aussitôt
# vérifiée avec la clé publique du dépôt : un secret qui n'est pas celui de la
# clé embarquée fait échouer la release au lieu de publier des paquets que les
# installations existantes refuseraient.
#
# Usage : sign-checksums.sh <dossier des livrables> [clé publique PEM]
set -euo pipefail

if (($# < 1 || $# > 2)); then
	echo "Usage : $(basename "$0") <dossier des livrables> [clé publique PEM]" >&2
	exit 2
fi
dist="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
public_key="${2:-$here/../../crates/manager/assets/release-signing.pub}"
readonly SIGNATURE_BYTES=64

[[ -n "${RELEASE_SIGNING_KEY:-}" ]] || { echo "RELEASE_SIGNING_KEY est vide ou absent : créer le secret du dépôt." >&2; exit 1; }
[[ -f "$dist/SHA256SUMS" ]] || { echo "$dist/SHA256SUMS introuvable." >&2; exit 1; }
[[ -f "$public_key" ]] || { echo "Clé publique introuvable : $public_key" >&2; exit 1; }

umask 077
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
printf '%s\n' "$RELEASE_SIGNING_KEY" >"$work/signing.key"

signature="$dist/SHA256SUMS.sig"
rm -f "$signature"
if ! openssl pkeyutl -sign -rawin -inkey "$work/signing.key" -in "$dist/SHA256SUMS" -out "$signature" 2>/dev/null; then
	echo "Signature impossible : RELEASE_SIGNING_KEY n'est pas une clé privée Ed25519 PEM." >&2
	rm -f "$signature"
	exit 1
fi
if [[ "$(stat -c %s "$signature")" -ne "$SIGNATURE_BYTES" ]] \
	|| ! openssl pkeyutl -verify -rawin -pubin -inkey "$public_key" -in "$dist/SHA256SUMS" -sigfile "$signature" >/dev/null 2>&1; then
	echo "La signature ne se vérifie pas avec $public_key : le secret n'est pas la clé de signature publiée." >&2
	rm -f "$signature"
	exit 1
fi
echo "SHA256SUMS signé et vérifié ($signature)."
