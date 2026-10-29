# syntax=docker/dockerfile:1
#
# Bergadeck — build reproductible et mode démonstration (`--demo`, appareil virtuel).
# L'exécution réelle passe par le service systemd utilisateur (USB, D-Bus et audio
# de la session), pas par ce conteneur.
#
# Versions alignées sur rust-toolchain.toml et .nvmrc (contrôlé par `make check-versions`) ;
# images épinglées par digest, rafraîchi par Dependabot.
#
# Le gestionnaire n'écoute que sur 127.0.0.1 (pas d'option d'écoute externe, par
# choix : contrôle strict Host/Origin) ; un `-p` ne le rendrait pas joignable.
# L'image s'utilise donc avec le réseau de l'hôte :
#   docker run --rm --network host bergadeck:dev
#   docker run --rm --network host bergadeck:dev --demo --no-tray --port 7979
# L'interface est alors sur http://127.0.0.1:7878/ (ou le port choisi).

# --------------------------------------------------------------------------- UI
FROM node:24-trixie-slim@sha256:89c5244bc22a9bc3287d89ebc7e49b9cb5bb84cd40e6175ac3421d5a9f68b95a AS ui

ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    CI=true
WORKDIR /src/ui
RUN corepack enable

# Dépendances d'abord, pour profiter du cache tant que le lockfile ne change pas.
COPY ui/package.json ui/pnpm-lock.yaml ui/pnpm-workspace.yaml ./
RUN --mount=type=cache,id=bergadeck-pnpm-store,target=/pnpm/store \
    pnpm install --frozen-lockfile --store-dir /pnpm/store

COPY ui/ ./
RUN pnpm run build

# ------------------------------------------------------------------------ Build
FROM rust:1.99.1-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS build

RUN apt-get update \
    && apt-get install --yes --no-install-recommends pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
# rust-toolchain.toml n'est pas copié : l'image fournit déjà la même version, et
# rustup tenterait sinon de télécharger les composants de dev (rustfmt, clippy).
COPY Cargo.toml Cargo.lock ./
COPY .cargo/ .cargo/
COPY crates/ crates/
# L'UI est embarquée dans le binaire : elle doit exister avant la compilation.
COPY --from=ui /src/ui/build/ ui/build/

RUN --mount=type=cache,id=bergadeck-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=bergadeck-target,target=/src/target \
    cargo build --release --locked -p bergadeck \
    && install -D -m 0755 target/release/bergadeck /out/bergadeck

# ---------------------------------------------------------------------- Runtime
FROM debian:trixie-slim@sha256:89c1ea91e897f0fe0a3731595330bc45b8bdef4047464a07116f891a94054b31

LABEL org.opencontainers.image.title="bergadeck" \
      org.opencontainers.image.description="Stream Deck manager for Linux (demo mode)" \
      org.opencontainers.image.licenses="GPL-3.0-or-later"

RUN apt-get update \
    && apt-get install --yes --no-install-recommends libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/bergadeck --uid 10001 \
       --user-group --shell /usr/sbin/nologin bergadeck

COPY --from=build /out/bergadeck /usr/local/bin/bergadeck

USER bergadeck
WORKDIR /home/bergadeck

ENTRYPOINT ["bergadeck"]
# Pas de session D-Bus dans le conteneur : pas de tray.
CMD ["--demo", "--no-tray"]
