# Bergadeck — commandes de développement, de vérification et d'installation.
#
# pnpm est lancé via Corepack (`corepack pnpm`, livré avec Node 24) depuis `ui/` :
# Corepack lit `packageManager` dans `ui/package.json` et exécute exactement la
# version épinglée, sans installation globale de pnpm. Un pnpm déjà dans le PATH
# peut être utilisé à la place : `make PNPM=pnpm <cible>`.
#
# Le chemin du dépôt contient des espaces : toute référence à un chemin est citée.

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help
MAKEFLAGS += --no-builtin-rules --no-print-directory

PNPM ?= corepack pnpm
CARGO ?= cargo
DOCKER ?= docker
# Version de Lighthouse CI utilisée en local et en CI (`make lighthouse`).
LHCI_VERSION ?= 0.15.1
DOCKER_IMAGE ?= bergadeck:dev

UI_DIR := ui
GENERATED_DIR := $(UI_DIR)/src/lib/generated
MANAGER_PACKAGE := bergadeck
PROTOCOL_PACKAGE := bergadeck-protocol

# Pas d'invite interactive de Corepack au premier téléchargement de pnpm.
export COREPACK_ENABLE_DOWNLOAD_PROMPT := 0

.PHONY: help dev run lint lint-rust lint-ui fmt test test-rust test-ui test-e2e \
	gen-types check-types check-versions check deny ui-deps ui-build build \
	lighthouse docker install uninstall clean

help: ## Affiche cette aide
	@printf 'Usage : make <cible>\n\n'
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

## ---------------------------------------------------------------- Développement

dev: ui-deps ## Gestionnaire en --demo + serveur de dev UI (Ctrl-C arrête les deux)
	@trap 'kill "$$manager_pid" "$$ui_pid" 2>/dev/null || true; wait' INT TERM; \
	$(CARGO) run -p $(MANAGER_PACKAGE) -- --demo & manager_pid=$$!; \
	(cd "$(UI_DIR)" && $(PNPM) run dev) & ui_pid=$$!; \
	wait

run: ## Lance le gestionnaire sur le matériel réel
	$(CARGO) run -p $(MANAGER_PACKAGE)

## ---------------------------------------------------------------- Qualité

lint: lint-rust lint-ui ## Lint et format en mode vérification (Rust + UI)

lint-rust: ## rustfmt --check + clippy (avertissements = erreurs)
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings

lint-ui: ui-deps ## Prettier + ESLint + svelte-check
	cd "$(UI_DIR)" && $(PNPM) run lint
	cd "$(UI_DIR)" && $(PNPM) run check

fmt: ui-deps ## Formate le code Rust et UI
	$(CARGO) fmt --all
	cd "$(UI_DIR)" && $(PNPM) run format

test: test-rust test-ui ## Tests unitaires et d'intégration (Rust + UI)

test-rust: ## cargo test sur tout le workspace
	$(CARGO) test --workspace --locked

test-ui: ui-deps ## Vitest
	cd "$(UI_DIR)" && $(PNPM) run test

test-e2e: ui-deps ## Tests Playwright (navigateurs : pnpm exec playwright install chromium)
	cd "$(UI_DIR)" && $(PNPM) run test:e2e

gen-types: ## Régénère les types TS du contrat dans ui/src/lib/generated
	rm -rf "$(GENERATED_DIR)"
	$(CARGO) test -p $(PROTOCOL_PACKAGE) --locked export_bindings

# Génère dans un dossier temporaire et compare : ne touche pas l'arbre de travail
# et détecte aussi bien les fichiers modifiés que les fichiers en trop ou manquants.
check-types: ## Vérifie que les types TS générés sont à jour
	@tmp_dir="$$(mktemp -d)"; trap 'rm -rf "$$tmp_dir"' EXIT; \
	TS_RS_EXPORT_DIR="$$tmp_dir" $(CARGO) test -p $(PROTOCOL_PACKAGE) --locked export_bindings >/dev/null; \
	if diff -r "$$tmp_dir" "$(GENERATED_DIR)"; then \
		echo 'Types TS à jour.'; \
	else \
		echo 'Types TS obsolètes : lancer « make gen-types » et committer le résultat.' >&2; \
		exit 1; \
	fi

# Les versions des images Docker doivent suivre rust-toolchain.toml et .nvmrc.
check-versions: ## Vérifie que le Dockerfile suit rust-toolchain.toml et .nvmrc
	@rust_version="$$(sed -nE 's/^channel *= *"([^"]+)"/\1/p' rust-toolchain.toml)"; \
	node_version="$$(tr -d '[:space:]' < .nvmrc)"; \
	status=0; \
	grep -qE "^FROM rust:$${rust_version//./\\.}-" Dockerfile \
		|| { echo "Dockerfile : image rust différente de rust-toolchain.toml ($$rust_version)" >&2; status=1; }; \
	grep -qE "^FROM node:$${node_version//./\\.}-" Dockerfile \
		|| { echo "Dockerfile : image node différente de .nvmrc ($$node_version)" >&2; status=1; }; \
	[ "$$status" -eq 0 ] && echo "Versions cohérentes (Rust $$rust_version, Node $$node_version)."; \
	exit "$$status"

deny: ## Licences, avis de sécurité et sources des dépendances Rust (cargo-deny)
	$(CARGO) deny --locked check

check: lint test check-types check-versions deny ## Toutes les vérifications locales

## ---------------------------------------------------------------- Build

ui-deps: ## Installe les dépendances UI si le lockfile a changé
	@if [ ! -f "$(UI_DIR)/node_modules/.modules.yaml" ] \
		|| [ "$(UI_DIR)/pnpm-lock.yaml" -nt "$(UI_DIR)/node_modules/.modules.yaml" ]; then \
		cd "$(UI_DIR)" && $(PNPM) install --frozen-lockfile; \
	fi

ui-build: ui-deps ## Construit l'UI statique (ui/build)
	cd "$(UI_DIR)" && $(PNPM) run build

# L'UI doit être construite avant le binaire, qui l'embarque (rust-embed).
build: ui-build ## UI puis binaire release qui l'embarque
	$(CARGO) build --release --locked -p $(MANAGER_PACKAGE)

lighthouse: ui-build ## Lighthouse CI (Accessibilité et Bonnes pratiques ≥ 0,9)
	npx --yes @lhci/cli@$(LHCI_VERSION) autorun

docker: ## Construit l'image Docker (build reproductible + mode --demo)
	$(DOCKER) build --tag "$(DOCKER_IMAGE)" .

## ---------------------------------------------------------------- Installation

install: build ## Installe binaire, service systemd utilisateur et règle udev
	"$(CURDIR)/packaging/install.sh"

uninstall: ## Désinstalle binaire, service et règle udev (garde la configuration)
	"$(CURDIR)/packaging/uninstall.sh"

clean: ## Supprime les artefacts de build (target, ui/build, rapports)
	$(CARGO) clean
	rm -rf "$(UI_DIR)/build" "$(UI_DIR)/.svelte-kit" .lighthouseci
