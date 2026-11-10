# HuBergamof — commandes de développement, de vérification et d'installation.
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
DOCKER_IMAGE ?= hubergamof:dev
# Paquets : version lue dans le Cargo.toml du workspace (la release l'y fixe
# d'après le tag avant de construire), binaire release, dossier de sortie.
PACKAGE_VERSION ?= $(shell sed -nE '/^\[workspace\.package\]/,/^\[/ s/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)
PACKAGE_DIR := packaging/package
DIST_DIR := dist
RELEASE_BINARY := target/release/hubergamof
ARCH_IMAGE := hubergamof-package-arch
UBUNTU_IMAGE := hubergamof-package-ubuntu
# Origines du serveur de dev Vite (port par défaut 5173), autorisées par le
# gestionnaire en plus des siennes (contrôle strict Host/Origin).
VITE_DEV_ORIGINS := http://localhost:5173 http://127.0.0.1:5173

UI_DIR := ui
GENERATED_DIR := $(UI_DIR)/src/lib/generated
MANAGER_PACKAGE := hubergamof
PROTOCOL_PACKAGE := hubergamof-protocol
MODULE_API_PACKAGE := hubergamof-module-api

# Pas d'invite interactive de Corepack au premier téléchargement de pnpm.
export COREPACK_ENABLE_DOWNLOAD_PROMPT := 0

.PHONY: help dev run lint lint-rust lint-ui lint-shell-exec lint-workflows fmt test test-rust test-ui test-e2e \
	test-scripts coverage coverage-rust coverage-ui test-docker \
	gen-types check-types check-versions check-node check-branch check deny ui-deps ui-build build \
	lighthouse docker install uninstall clean package package-deb package-arch package-lint package-smoke

help: ## Affiche cette aide
	@printf 'Usage : make <cible>\n\n'
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

## ---------------------------------------------------------------- Développement

# Sans tray : `make dev` ne doit pas ajouter d'icône à côté d'un gestionnaire installé.
dev: ui-deps ## Gestionnaire en --demo + serveur de dev UI (Ctrl-C arrête les deux)
	@trap 'kill "$$manager_pid" "$$ui_pid" 2>/dev/null || true; wait' INT TERM; \
	$(CARGO) run -p $(MANAGER_PACKAGE) -- --demo --no-tray \
		$(foreach origin,$(VITE_DEV_ORIGINS),--dev-origin $(origin)) & manager_pid=$$!; \
	(cd "$(UI_DIR)" && $(PNPM) run dev) & ui_pid=$$!; \
	wait

run: ## Lance le gestionnaire sur le matériel réel
	$(CARGO) run -p $(MANAGER_PACKAGE)

## ---------------------------------------------------------------- Qualité

lint: lint-rust lint-ui lint-shell-exec ## Lint et format en mode vérification (Rust + UI)

lint-rust: ## rustfmt --check + clippy (avertissements = erreurs)
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings
	@# L'API des modules seule et sans features : dans le workspace, l'unification
	@# des features masquerait un usage de `tokio::sync`, réservé à `testing`.
	$(CARGO) clippy -p $(MODULE_API_PACKAGE) --no-default-features --locked -- -D warnings

# Garde de sécurité : aucun module ne doit lancer `sh -c` (injection depuis la
# configuration). Chemins analysés surchargeables : `make lint-shell-exec SHELL_EXEC_ROOTS=…`.
SHELL_EXEC_ROOTS ?= crates
lint-shell-exec: ## Interdit tout lancement de shell dans le code Rust hors tests
	"$(CURDIR)/.github/scripts/check-shell-exec.sh" $(SHELL_EXEC_ROOTS)

# Audit de sécurité des workflows GitHub (injections, permissions, épinglage) ;
# hors ligne : pas de jeton. Outil installé à part (CI : taiki-e/install-action).
lint-workflows: ## zizmor sur .github (bloquant)
	zizmor --offline --format plain .github

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

# Cliquet : couverture mesurée moins 2 points, relevée quand des tests s'ajoutent,
# jamais baissée sans accord. Côté UI, le plancher est dans ui/vite.config.ts.
RUST_COVERAGE_FLOOR_LINES := 91
RUST_COVERAGE_FLOOR_FUNCTIONS := 89
RUST_COVERAGE_FLOOR_REGIONS := 91

coverage: coverage-rust coverage-ui ## Couverture des tests (Rust + UI), en échec sous le plancher

coverage-rust: ## Couverture Rust (cargo-llvm-cov, composant llvm-tools-preview)
	$(CARGO) llvm-cov --workspace --locked --summary-only \
		--fail-under-lines $(RUST_COVERAGE_FLOOR_LINES) \
		--fail-under-functions $(RUST_COVERAGE_FLOOR_FUNCTIONS) \
		--fail-under-regions $(RUST_COVERAGE_FLOOR_REGIONS)
	@# cargo llvm-cov ne lance pas les doctests sur stable (exemple de module de module-api).
	$(CARGO) test --workspace --locked --doc

coverage-ui: ui-deps ## Couverture UI (Vitest, v8)
	cd "$(UI_DIR)" && $(PNPM) run test:coverage

test-docker: ## Module Docker contre le vrai Docker de la machine (feature docker-it)
	$(CARGO) test -p hubergamof-modules --locked --features docker-it --test docker_engine

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

# pnpm 12 ne démarre pas avec le Corepack des Node < 24.12 : message clair avant.
check-node: ## Vérifie que Node respecte le minimum de ui/package.json (engines)
	"$(CURDIR)/.github/scripts/check-node-version.sh"

# Obligatoire : la branche générée par l'outil (claude/<adjectif>-<nom>-<hasard>) doit être renommée.
check-branch: ## Vérifie le nom de la branche courante (git branch -m pour renommer)
	"$(CURDIR)/.github/scripts/check-branch-name.sh"

test-scripts: ## Tests des scripts de contrôle (.github/scripts)
	"$(CURDIR)/.github/scripts/test-control-scripts.sh"

deny: ## Licences, avis de sécurité et sources des dépendances Rust (cargo-deny)
	$(CARGO) deny --locked check

# `coverage` plutôt que `test` : mêmes tests, plus les planchers vérifiés en CI.
check: check-branch lint test-scripts coverage check-types check-versions deny ## Toutes les vérifications locales

## ---------------------------------------------------------------- Build

ui-deps: check-node ## Installe les dépendances UI si le lockfile a changé
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

## ---------------------------------------------------------------- Paquets

package: package-deb package-arch ## Paquets .deb (Ubuntu) et pacman (Manjaro) dans dist/ (après make build)

package-deb: ## Paquet .deb (dpkg-deb, dpkg-shlibdeps)
	"$(CURDIR)/$(PACKAGE_DIR)/build-deb.sh" "$(RELEASE_BINARY)" "$(PACKAGE_VERSION)" "$(DIST_DIR)"

# makepkg refuse root : conteneur Arch, avec un dossier de sortie accessible à son
# utilisateur (le dossier lui-même seulement ; le .deb reste à l'utilisateur courant).
package-arch: ## Paquet pacman (.pkg.tar.zst, makepkg dans un conteneur Arch)
	mkdir -p "$(DIST_DIR)"
	chmod 0777 "$(DIST_DIR)"
	$(DOCKER) build --quiet --tag "$(ARCH_IMAGE)" "$(PACKAGE_DIR)/arch"
	$(DOCKER) run --rm -v "$(CURDIR):/src:ro" -v "$(CURDIR)/$(DIST_DIR):/dist" "$(ARCH_IMAGE)" \
		"/src/$(PACKAGE_DIR)/build-arch.sh" "/src/$(RELEASE_BINARY)" "$(PACKAGE_VERSION)" /dist

package-lint: ## lintian sur le .deb (erreurs bloquantes)
	lintian --fail-on error "$(DIST_DIR)/hubergamof_$(PACKAGE_VERSION)_amd64.deb"

package-smoke: ## Installe chaque paquet dans un conteneur propre (Ubuntu, Arch) puis le désinstalle
	$(DOCKER) build --quiet --tag "$(UBUNTU_IMAGE)" "$(PACKAGE_DIR)/ubuntu"
	$(DOCKER) build --quiet --tag "$(ARCH_IMAGE)" "$(PACKAGE_DIR)/arch"
	$(DOCKER) run --rm --user root -v "$(CURDIR):/src:ro" -v "$(CURDIR)/$(DIST_DIR):/dist:ro" "$(UBUNTU_IMAGE)" \
		"/src/$(PACKAGE_DIR)/smoke-test.sh" deb "$(PACKAGE_VERSION)" /dist
	$(DOCKER) run --rm --user root -v "$(CURDIR):/src:ro" -v "$(CURDIR)/$(DIST_DIR):/dist:ro" "$(ARCH_IMAGE)" \
		"/src/$(PACKAGE_DIR)/smoke-test.sh" arch "$(PACKAGE_VERSION)" /dist

clean: ## Supprime les artefacts de build (target, ui/build, rapports)
	$(CARGO) clean
	rm -rf "$(UI_DIR)/build" "$(UI_DIR)/.svelte-kit" .lighthouseci "$(DIST_DIR)"
