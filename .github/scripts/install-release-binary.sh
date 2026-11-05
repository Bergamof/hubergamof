#!/usr/bin/env bash
# Télécharge une archive depuis les releases GitHub, vérifie sa somme SHA-256
# contre la valeur épinglée dans le workflow (pas contre un fichier de sommes de
# la même release, qu'un attaquant maître de la release remplacerait aussi), puis
# installe le binaire dans un dossier ajouté au PATH du job.
# Usage : install-release-binary.sh <owner/repo> <tag> <archive> <sha256> <binaire>
set -euo pipefail

readonly REPOSITORY="$1" TAG="$2" ARCHIVE="$3" SHA256="$4" BINARY="$5"
readonly BASE_URL="https://github.com/$REPOSITORY/releases/download/$TAG"
readonly INSTALL_DIR="${RUNNER_TEMP:?}/bin"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
cd "$work_dir"

curl --fail --silent --show-error --location --remote-name "$BASE_URL/$ARCHIVE"
echo "$SHA256  $ARCHIVE" | sha256sum --check --strict

tar --extract --gzip --file "$ARCHIVE" "$BINARY"
install -D -m 0755 "$BINARY" "$INSTALL_DIR/$BINARY"
echo "$INSTALL_DIR" >>"${GITHUB_PATH:?}"
