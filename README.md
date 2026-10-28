# Bergadeck

Gestionnaire de Stream Deck pour Linux, à base de modules, avec une interface de configuration web locale synchronisée en temps réel.

- **Gestionnaire** (Rust) : pilote les appareils (touches, molettes, bande tactile), exécute les modules, sert l'interface et un WebSocket sur `127.0.0.1:7878`, icône dans la barre système.
- **Interface** (SvelteKit) : détecte les appareils, configure l'accueil, les modules et les molettes ; les changements s'appliquent immédiatement sur l'appareil.
- **Modules intégrés** : Horloge, Navigation, Audio, Pomodoro, Docker, IntelliJ, Claude.

Cible actuelle : Stream Deck+ sous KDE Plasma et GNOME (Wayland). Toute la gamme Stream Deck est prise en charge par conception.

> Projet en cours d'initialisation : le gestionnaire ne pilote pas encore les appareils.

## Prérequis

| Outil | Rôle | Installation |
|---|---|---|
| [rustup](https://rustup.rs/) | installe automatiquement Rust 1.99.1 (épinglé dans `rust-toolchain.toml`) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` ou paquet `rustup` |
| [nvm](https://github.com/nvm-sh/nvm) | Node 24 LTS (épinglé dans `.nvmrc`) | voir le lien |
| Corepack (fourni avec Node 24) | pnpm à la version épinglée dans `ui/package.json` | `corepack enable pnpm` |
| Bibliothèques système | accès USB/HID | Arch : `sudo pacman -S hidapi systemd-libs pkgconf` ; Ubuntu : `sudo apt install pkg-config libudev-dev libhidapi-dev` |
| PipeWire (`pactl`, `wpctl`) | module Audio | présent par défaut sur Arch/KDE et Ubuntu 24.04 |
| Docker (utilisateur dans le groupe `docker`) | module Docker, `make docker` | [docs.docker.com](https://docs.docker.com/engine/install/) |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) 0.20 | `make deny` / `make check` | `cargo install --locked cargo-deny` |
| GNOME uniquement : extension AppIndicator | icône de barre système | activée par défaut sur Ubuntu |

## Activation automatique de l'environnement

### Rust (rustup)
1. Rien à installer à la main : à la première commande `cargo` dans le dossier, rustup lit `rust-toolchain.toml` et installe la 1.99.1 avec `rustfmt` et `clippy`.
2. Le basculement est automatique dans ce dossier (et ses sous-dossiers).
3. Vérification : `rustc --version` → `rustc 1.99.1 …`.

### Node (nvm)
1. Installation de la version épinglée : `nvm install` (lit `.nvmrc`).
2. Activation ponctuelle : `nvm use`.
3. Basculement automatique : ajouter dans `~/.bashrc` (ou `~/.zshrc`), après le chargement de nvm, le hook « deeper shell integration » de la [documentation nvm](https://github.com/nvm-sh/nvm#deeper-shell-integration) (fonction `cdnvm` pour bash, `load-nvmrc` + `add-zsh-hook chpwd` pour zsh).
4. Vérification : `node -v` → `v24.x` ; `corepack pnpm -v` (depuis `ui/`) → `12.x`.

## Commandes

Toutes passent par le `Makefile` (`make help` liste tout) :

| Commande | Effet |
|---|---|
| `make dev` | gestionnaire en mode `--demo` (appareil virtuel) + serveur de dev de l'UI avec rechargement à chaud ; Ctrl-C arrête les deux |
| `make run` | gestionnaire sur le matériel réel |
| `make check` | toutes les vérifications locales (lint, tests, contrat TS, versions, cargo-deny) |
| `make lint` / `make fmt` | lint en vérification / formatage |
| `make test` / `make test-e2e` | tests Rust + Vitest / tests Playwright (`corepack pnpm -C ui exec playwright install chromium` au préalable ; aucun gestionnaire ne doit tourner sur le port 7878) |
| `make gen-types` | régénère les types TypeScript du contrat (`ui/src/lib/generated/`) |
| `make build` | UI puis binaire release `target/release/bergadeck` qui l'embarque |
| `make docker` | image de build reproductible (mode `--demo`, pas l'usage normal) |
| `make lighthouse` | audit Lighthouse (Accessibilité et Bonnes pratiques ≥ 90) |
| `make install` / `make uninstall` | installe/retire le binaire (`~/.local/bin`), le service systemd utilisateur et la règle udev |

## Installation sur le poste

```sh
make build
make install
```

`make install` copie le binaire, installe le service utilisateur `bergadeck.service` (démarré avec la session) et, après confirmation, la règle udev `packaging/udev/70-bergadeck.rules` (avec `sudo`) qui donne accès aux Stream Deck sans être root. Débrancher puis rebrancher l'appareil ensuite. Options : `packaging/install.sh --no-udev`, `--yes`.

La configuration des appareils est, pour l'instant, stockée dans le dépôt : `config/devices/<numéro de série>/` (ignoré par git ; un exemple versionné est dans `config/example/`). L'interface permet de l'exporter et de l'importer en `.zip`.

Interface : <http://127.0.0.1:7878> (ou via l'icône de la barre système).

## CI

`.github/workflows/ci.yml` vérifie chaque pull request (et chaque push sur `main`) : lint, tests, contrat TypeScript à jour, tests e2e, build, image Docker, sécurité (cargo-deny, `pnpm audit`, gitleaks, Trivy) et Lighthouse. Un job final commente la PR avec un compte rendu.

Recommandé : protéger `main` en rendant obligatoires les jobs `lint`, `test`, `contract`, `e2e`, `build`, `docker`, `security` et `lighthouse`.

## Licence

[GPL-3.0-or-later](LICENSE). La crate `crates/module-api` (contrat des modules) est sous [MPL-2.0](crates/module-api/LICENSE) afin que les auteurs de modules restent libres de leur licence.
