#!/usr/bin/env bash
# Télécharge un binaire depuis les releases GitHub, vérifie sa somme SHA-256 avec
# le fichier de sommes publié, puis l'installe dans un dossier ajouté au PATH du job.
# Usage : install-release-binary.sh <owner/repo> <tag> <archive> <checksums> <binaire>
set -euo pipefail

readonly REPOSITORY="$1" TAG="$2" ARCHIVE="$3" CHECKSUMS="$4" BINARY="$5"
readonly BASE_URL="https://github.com/$REPOSITORY/releases/download/$TAG"
readonly INSTALL_DIR="${RUNNER_TEMP:?}/bin"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
cd "$work_dir"

curl --fail --silent --show-error --location --remote-name "$BASE_URL/$ARCHIVE"
curl --fail --silent --show-error --location --remote-name "$BASE_URL/$CHECKSUMS"
grep -F " $ARCHIVE" "$CHECKSUMS" | sha256sum --check --strict

tar --extract --gzip --file "$ARCHIVE" "$BINARY"
install -D -m 0755 "$BINARY" "$INSTALL_DIR/$BINARY"
echo "$INSTALL_DIR" >>"${GITHUB_PATH:?}"
