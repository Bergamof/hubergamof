# HuBergamof

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.svg">
  <img src="docs/assets/wordmark-light.svg" alt="HuBergamof" height="48">
</picture>

Gestionnaire de Stream Deck pour Linux, à base de modules, avec une interface de configuration web locale synchronisée en temps réel.

- **Gestionnaire** (Rust) : pilote les appareils (touches, molettes, bande tactile), exécute les modules, sert l'interface et un WebSocket sur `127.0.0.1:7878`, icône dans la barre système.
- **Interface** (SvelteKit) : détecte les appareils, configure l'accueil, les modules et les molettes ; les changements s'appliquent immédiatement sur l'appareil.
- **Modules intégrés** : Horloge, Navigation, Audio, Pomodoro, Docker, IntelliJ, Claude.

Cible actuelle : Stream Deck+ sous KDE Plasma et GNOME (Wayland). Toute la gamme Stream Deck est prise en charge par conception.

> Projet en cours de développement : le gestionnaire détecte et pilote les appareils, mais les modules et l'interface ne sont pas encore branchés.

## Prérequis

| Outil | Rôle | Installation |
|---|---|---|
| [rustup](https://rustup.rs/) | installe automatiquement Rust 1.99.1 (épinglé dans `rust-toolchain.toml`) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` ou paquet `rustup` |
| [nvm](https://github.com/nvm-sh/nvm) | Node 24 LTS, **24.12 minimum** (`.nvmrc`, `engines` de `ui/package.json`) | voir le lien |
| Corepack (fourni avec Node ≥ 24.12) | pnpm à la version épinglée dans `ui/package.json` | `corepack enable pnpm` |
| Bibliothèques système | accès USB/HID | Arch : `sudo pacman -S base-devel systemd-libs pkgconf` ; Ubuntu : `sudo apt install build-essential pkg-config libudev-dev` |
| `pactl` ≥ 16 (serveur PipeWire ou PulseAudio) | module Audio | Arch : `libpulse` (présent avec Plasma) ; Ubuntu 24.04 ou plus : `sudo apt install pulseaudio-utils` (non installé par défaut ; Ubuntu 22.04 fournit `pactl` 15, trop ancien) |
| Docker (utilisateur dans le groupe `docker`) | module Docker, `make docker` | [docs.docker.com](https://docs.docker.com/engine/install/) |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) 0.20 | `make deny` / `make check` | `cargo install --locked cargo-deny` |
| [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) 0.9 | `make coverage` (composant `llvm-tools-preview` installé par rustup) | `cargo install --locked cargo-llvm-cov` |
| [zizmor](https://docs.zizmor.sh/) 1.31 (facultatif) | `make lint-workflows` (audit des workflows, bloquant en CI) | `cargo install --locked zizmor` |
| GNOME uniquement : extension AppIndicator | icône de barre système | activée par défaut sur Ubuntu |

## Activation automatique de l'environnement

### Rust (rustup)
1. Rien à installer à la main : à la première commande `cargo` dans le dossier, rustup lit `rust-toolchain.toml` et installe la 1.99.1 avec `rustfmt`, `clippy` et `llvm-tools-preview` (couverture).
2. Le basculement est automatique dans ce dossier (et ses sous-dossiers).
3. Vérification : `rustc --version` → `rustc 1.99.1 …`.

### Node (nvm)
1. Installation de la version épinglée : `nvm install` (lit `.nvmrc`).
2. Activation ponctuelle : `nvm use`.
3. Basculement automatique : ajouter dans `~/.bashrc` (ou `~/.zshrc`), après le chargement de nvm, le hook « deeper shell integration » de la [documentation nvm](https://github.com/nvm-sh/nvm#deeper-shell-integration) (fonction `cdnvm` pour bash, `load-nvmrc` + `add-zsh-hook chpwd` pour zsh).
4. Vérification : `node -v` → `v24.12` ou plus ; `corepack pnpm -v` (depuis `ui/`) → `12.x`.

### Dépannage pnpm
- `Cannot find module …/corepack/v1/pnpm/12.…/bin/pnpm.cjs` : Node trop ancien (< 24.12, Corepack < 0.34.5, qui ne sait pas lancer pnpm 12). `make` le détecte (`make check-node`). Remède : `nvm install && nvm use`, puis `rm -rf ~/.cache/node/corepack/v1/pnpm` (le téléchargement raté laisse un cache incomplet que même un Corepack récent réutilise) et `corepack enable pnpm` (le raccourci est propre à chaque version de Node).
- `getaddrinfo ENOTFOUND registry.npmjs.org` : pas de réseau (DNS, VPN, proxy) ; Corepack télécharge pnpm une fois depuis le registre npm. Derrière un proxy : `HTTPS_PROXY` ; registre interne : `COREPACK_NPM_REGISTRY`.

## Commandes

Toutes passent par le `Makefile` (`make help` liste tout) :

| Commande | Effet |
|---|---|
| `make dev` | gestionnaire en mode `--demo` (appareil virtuel) + serveur de dev de l'UI avec rechargement à chaud ; Ctrl-C arrête les deux |
| `make run` | gestionnaire sur le matériel réel |
| `make check` | toutes les vérifications locales (nom de branche, lint, tests avec planchers de couverture, contrat TS, versions, cargo-deny) |
| `make lint` / `make fmt` | lint en vérification / formatage |
| `make check-branch` | nom de branche parlant, obligatoire (`[claude/]<type>/<mots-en-kebab>`, ex. `fix/node-minimum-version`) : renommer avec `git branch -m` |
| `make test` / `make test-e2e` | tests Rust + Vitest (composants Svelte dans Chromium, mode navigateur) / tests Playwright : chaque test lance son propre `hubergamof --demo` (binaire debug compilé au démarrage, port libre, config neuve) ; le reste tourne sur `vite preview` (`corepack pnpm -C ui exec playwright install chromium` au préalable ; aucun gestionnaire ne doit tourner sur le port 7878) |
| `make coverage` | tests Rust et Vitest instrumentés, en échec sous le plancher de couverture (mesure − 2 points, jamais baissé) |
| `make test-docker` | module Docker contre le vrai Docker de la machine (conteneurs `hubergamof-it-*` créés puis supprimés) |
| `make gen-types` | régénère les types TypeScript du contrat (`ui/src/lib/generated/`) |
| `make build` | UI puis binaire release `target/release/hubergamof` qui l'embarque |
| `make docker` | image de build reproductible (mode `--demo`, pas l'usage normal) |
| `make lighthouse` | audit Lighthouse de l'UI servie par `hubergamof --demo` (port 7881) : Accessibilité, Bonnes pratiques et Performance ≥ 90 |
| `make package` / `make package-lint` / `make package-smoke` | paquets `.deb` et pacman dans `dist/` (après `make build` ; Docker requis), `lintian`, test d'installation en conteneurs |
| `make install` / `make uninstall` | installe/retire le binaire (`~/.local/bin`), le service systemd utilisateur et la règle udev |

## Installation par paquet

Les [releases GitHub](https://github.com/Bergamof/hubergamof/releases) fournissent un paquet par distribution :

```sh
# Ubuntu 24.04+, Debian 12+
sudo apt install ./hubergamof_X.Y.Z_amd64.deb
# Manjaro, Arch Linux
sudo pacman -U hubergamof-X.Y.Z-1-x86_64.pkg.tar.zst
```

Le paquet installe les dépendances nécessaires (dont `pactl` et un serveur de son pour le module Audio), la règle udev et le service utilisateur, qu'il démarre dans les sessions graphiques ouvertes. Docker est facultatif (module Docker : l'utilisateur doit être dans le groupe `docker`). La configuration est dans `~/.config/hubergamof`. Détails, numérotation et fabrication des releases : [`docs/release.md`](docs/release.md).

## Installation depuis les sources

```sh
make build
make install
```

`make install` copie le binaire, installe le service utilisateur `hubergamof.service` (démarré avec la session) et, après confirmation, la règle udev `packaging/udev/70-hubergamof.rules` (avec `sudo`) qui donne accès aux Stream Deck sans être root. Débrancher puis rebrancher l'appareil ensuite. Options : `packaging/install.sh --no-udev`, `--yes`.

La configuration des appareils est, pour l'instant, stockée dans le dépôt : `config/devices/<numéro de série>/` (ignoré par git ; un exemple versionné est dans `config/example/`). L'interface permet de l'exporter et de l'importer en `.zip`.

Interface : <http://127.0.0.1:7878> (ou via l'icône de la barre système).

Le lanceur de bureau **HuBergamof** (menu des applications, posé par le paquet ou par `make install`) ouvre l'interface et, si le gestionnaire ne tourne plus (quitté depuis la barre système, service arrêté ou abandonné par systemd après des échecs répétés), relance d'abord le service. En ligne de commande : `hubergamof open` (`hubergamof open --port <port>` si le port a été changé).

## Vie privée

Pour signaler qu'une application suivie (Claude Desktop, IntelliJ) demande votre attention, le gestionnaire observe les notifications de bureau de la session (`org.freedesktop.Notifications`). Seules celles envoyées par ces applications, reconnues par l'exécutable de l'expéditeur (et, pour claude.ai ouvert dans Brave, par le site que le navigateur indique : les notifications des autres sites sont écartées), sont retenues, pour mettre leur touche en attention : leur texte n'est ni affiché ni conservé ; les autres sont ignorées sans être conservées ni journalisées. Les fenêtres sont suivies par un script KWin qui ne transmet que celles des applications suivies. Un journal de débogage de zbus (`RUST_LOG=zbus=trace`) afficherait en revanche les messages du bus : à éviter hors diagnostic.

## CI

`.github/workflows/ci.yml` vérifie chaque pull request (et chaque push sur `main`, plus une fois par semaine) : lint (dont zizmor), tests avec couverture, contrat TypeScript à jour, tests e2e, build, image Docker, intégration avec un vrai Docker (si la PR touche le module), sécurité (cargo-deny, `pnpm audit`, gitleaks, Trivy) et Lighthouse. Un job `package` construit et installe les paquets en conteneur. Un job final commente la PR avec un compte rendu.

`.github/workflows/release.yml` (lancé à la main, sur `main`) publie une release : voir [`docs/release.md`](docs/release.md).

Recommandé : protéger `main` en rendant obligatoires les jobs `lint`, `test`, `contract`, `e2e`, `build`, `package`, `docker`, `docker-it`, `security` et `lighthouse`.

## Licence

[GPL-3.0-or-later](LICENSE). La crate `crates/module-api` (contrat des modules) est sous [MPL-2.0](crates/module-api/LICENSE) afin que les auteurs de modules restent libres de leur licence.

Le binaire embarque la police Noto Sans (graisses SemiBold et Light), sous [SIL Open Font License 1.1](crates/manager/assets/fonts/OFL.txt) ; provenance et empreintes dans [`crates/manager/assets/fonts/README.md`](crates/manager/assets/fonts/README.md).
