# HuBergamof

Gestionnaire de Stream Deck (cible : Stream Deck+, toute la gamme par conception) pour Linux, à base de modules, avec une interface de configuration web locale synchronisée en temps réel.

## Stack
- Gestionnaire : Rust 1.99.1 (édition 2024), tokio, axum (HTTP + WebSocket), `elgato-streamdeck` (+ `hidapi`), `reqwest` (rustls ; mises à jour), `ed25519-dalek` (vérification de la signature des releases), tray SNI/dbusmenu maison sur `zbus` 6, `zbus` (D-Bus), `bollard` (Docker), `image` + `resvg`, `zip` (Deflate seul, export), `rust-embed`, `ts-rs`.
- Interface : SvelteKit 3.1 (`adapter-static`), Svelte 5.57, Vite 8, Tailwind 4.3 (polices embarquées `@fontsource-variable/*`), TypeScript 6 (TS 7 exclu par svelte-check/typescript-eslint), pnpm 12.16 (Corepack), Node 24 LTS ; ESLint 10 + Prettier 3, Vitest 5 (+ mode navigateur, couverture v8), Playwright 1.65 (planchers de couverture UI : lignes 86, fonctions 85, branches 77, instructions 89).
- Persistance : fichiers JSON par appareil (`config/devices/<serial>/`), pas de base de données.
- Déploiement : un seul binaire `hubergamof` (UI embarquée), service utilisateur systemd + règle udev.

## Environnement
- Versions épinglées dans : `rust-toolchain.toml`, `.nvmrc`, `ui/package.json` (`packageManager`, `engines`) — source de vérité, lue aussi par la CI. `packageManager` porte l'empreinte sha512 de pnpm : pour monter pnpm, `corepack use pnpm@<version>` depuis `ui/` (écrit version et empreinte).
- Activation (manuelle et auto) : voir la section dédiée du `README.md`.
- Toute montée de version d'outil se fait dans ces fichiers ET dans le `Dockerfile` (et le README si la démarche change), dans le même commit.
- Node ≥ 24.12 (`engines`) : le Corepack des Node plus anciens (< 0.34.5) ne sait pas lancer pnpm 12 (`Cannot find module …/pnpm.cjs`, cache `~/.cache/node/corepack/v1/pnpm/<version>` incomplet à effacer) ; `make check-node` (prérequis de `ui-deps`) le détecte. Minimum lu depuis `engines` par `.github/scripts/check-node-version.sh`.
- Prérequis système : compilateur C + `pkg-config` + `libudev-dev` (hidapi compilé statiquement par `elgato-streamdeck`, `libhidapi` système inutile), `pactl` ≥ 16 (module Audio ; Ubuntu : paquet `pulseaudio-utils`, absent par défaut ; `wpctl` inutilisé), Docker (utilisateur dans le groupe `docker`), règle udev `packaging/udev/70-hubergamof.rules` (Stream Deck + interface HID++ du récepteur Logi Bolt seulement, `docs/security.md`).

## Commandes (Makefile racine)
- Tout vérifier : `make check` (lint + tests instrumentés avec planchers de couverture + doctests + contrat TS à jour)
- Lint/format : `make lint` (+ `lint-shell-exec` : aucun `sh -c`/`bash` hors tests, `.github/scripts/check-shell-exec.sh` ; `lint-rust` vérifie aussi `module-api` seul sans features) / `make fmt`
- Tests : `make test` (Rust : `cargo test -p <crate> <nom>` ; UI : `pnpm -C ui test -- <fichier>`, composants dans `*.browser.spec.ts`) ; e2e : `make test-e2e` ; couverture : `make coverage` (plancher en cliquet : mesure − 2 points, Rust dans le `Makefile`, UI dans `ui/vite.config.ts`, jamais baissé) ; vrai Docker : `make test-docker` (feature `docker-it`)
- Types TS du contrat : `make gen-types` (ne doit produire aucun diff en CI)
- Build : `make build` (UI puis binaire release qui l'embarque)
- Lancement local : `make dev` (gestionnaire en `--demo` + `pnpm dev` avec proxy) ; matériel réel : `make run`
- Paquets : `make package` (`.deb` + pacman dans `dist/`, après `make build`, Docker requis), `package-lint`, `package-smoke`. Release : voir `docs/release.md` (manuelle sur `main`, version déduite des commits, 1.0.0 au début, niveau minimal au choix `auto|patch|minor|major` qui ne rétrograde jamais).
- Aussi : `make check-node` / `make check-branch` / `make test-scripts` (scripts de `.github/scripts/` + leur auto-test), `make check-versions` (Dockerfile ↔ `rust-toolchain.toml`/`.nvmrc`), `make deny`, `make lighthouse`, `make lint-workflows` (zizmor), sous-cibles `lint-rust|lint-ui|test-rust|test-ui|coverage-rust|coverage-ui|ui-build` (appelées telles quelles par la CI).
- pnpm : `PNPM ?= corepack pnpm` lancé depuis `ui/` (Corepack lit `packageManager` du dossier courant) ; `make PNPM=pnpm …` en CI.

## CI
- `.github/workflows/ci.yml` : sur chaque PR, push sur `main` et chaque semaine → `lint` (+ actionlint, shellcheck, zizmor bloquant), `test` (tests instrumentés + couverture), `contract`, `e2e` (`failOnFlakyTests`), `build`, `package` (paquets + installation en conteneurs), `docker` (+ Trivy image, HIGH/CRITICAL sans correctif publiés en information), `docker-it` (vrai Docker, sur PR seulement si le module est touché), `security` (cargo-deny, pnpm audit, gitleaks, Trivy fs), `lighthouse` (A11y, Bonnes pratiques, Perf ≥ 0,9) → job `report` (`if: always()`) qui commente la PR. Actions composites : `.github/actions/setup-rust|setup-ui`.
- Toutes les actions épinglées par SHA ; Dependabot ne suit pas `LHCI_VERSION`, gitleaks, actionlint (SHA-256 épinglé), zizmor ni cargo-llvm-cov (à la main, variables `env` du workflow).
- Toute nouvelle vérification est ajoutée à la CI ET aux `needs` du job `report`.
- Dépôt GitHub `Bergamof/hubergamof` : la CI s'exécute sur chaque PR (en session cloud, suivi via les outils MCP GitHub, pas de `gh`). « Terminé localement, CI non vérifiée » seulement si les runs sont inaccessibles.

## Équipe d'agents (`.claude/agents/`)
- Session principale (Chef Bouillon) : plan, briefs aux spécialistes, arbitrage et **toute écriture** (code Rust, UI, CI/CD, packaging, docs, `.claude/**`). Pas de sous-agent de développement (décision d'Adrien, 2026-11-01).
- Conventions et pièges par techno : `.claude/rules/rust.md`, `ui.md`, `ci-packaging.md` (chargés selon les fichiers touchés).
- Spécialistes (aucun droit d'écriture, `model:` explicite jamais `inherit`), rapports uniquement :

| Agent | Modèle | Écrit | Rôle |
|---|---|---|---|
| `gardien-du-phare` | sonnet | non | UX & accessibilité (Lighthouse ≥ 90 A11y & Bonnes pratiques) |
| `turbo-escargot` | sonnet | non | performances |
| `cerbere-le-paranoiaque` | opus | non | sécurité & CVE |
| `maniaque-du-plumeau` | sonnet | non | architecture & clean code |
| `capitaine-assertion` | sonnet | non | stratégie de tests |

## Méthode de travail
- Tâche non triviale : d'abord un plan d'action (choix techniques et fonctionnels justifiés, étapes, livrables, spécialistes mobilisés, risques), puis attendre le « go » d'Adrien avant de coder. Écart significatif en cours de route → resoumettre.
- Les spécialistes rendent des rapports (sévérité, localisation, recommandation) que la session principale doit suivre.
- Boucle : dev → rapports des spécialistes concernés → corrections → nouvelle passe, jusqu'à zéro bloquant/majeur. Désaccord avec un spécialiste : enjeu significatif → Adrien.
- Ne solliciter que les spécialistes concernés par la modification (chaque appel repart de zéro et coûte des tokens). Garder le contexte léger : filtrer les sorties de build/tests (erreurs, résumé) plutôt que les afficher en entier.
- Réutiliser les spécialistes existants ; jamais de suppression d'agent sans accord d'Adrien. Modèle fixé dans le frontmatter (`model:`) : opus seulement là où une omission coûte cher et se voit mal ensuite (sécurité), sonnet pour le reste (architecture passée de opus à sonnet, décision d'Adrien 2026-11-18). Piège appris → l'ajouter au fichier `.claude/rules/` de la techno (ou à la définition du spécialiste concerné).

## Définition de « done » (sauf indication contraire d'Adrien)
- Terminé seulement quand toutes les CI déclenchées par les modifications sont vertes sur le dernier commit poussé. Des tests locaux verts ne suffisent pas.
- Après chaque push / ouverture de PR : suivre les runs jusqu'à la fin (outils de suivi de la session, ou `gh` / API GitHub sur le SHA poussé).
- Échec : lire les logs, identifier la cause racine, corriger, repousser, re-surveiller.
- Interdit pour obtenir le vert, sauf accord explicite d'Adrien : skipper/désactiver un test, assouplir le lint, `continue-on-error`, supprimer un job, baisser un seuil (couverture, Lighthouse, sévérité CVE).
- Échec non lié (flaky, infra, service externe) : relancer une fois, puis signaler avec le diagnostic. Après ~3 échecs sur le même problème : s'arrêter et exposer diagnostic et pistes.
- CI inaccessible : annoncer « terminé localement, CI non vérifiée », jamais « done ».
- CI verte : annoncer la fin avec le lien du run, puis arrêter de surveiller la PR (pas d'attente des commentaires/reviews, sauf demande).

## Architecture
Détail complet (par crate, flux, API) : `docs/architecture.md` ; modules : `docs/modules.md` ; bureau (KWin, zbus, Teams, verrouillage) : `docs/integrations-desktop.md`. Flux : appareil → `DeckBackend` → moteur de pages → module → rendu → appareil ; toute modification d'état est diffusée aux clients WebSocket.
- `crates/protocol/` (`hubergamof-protocol`) — contrat WebSocket (`ClientMessage`/`ServerMessage`, taggés `type`, camelCase ; dont `playHaptic` pour essayer une vibration), configuration + `DeviceConfig::validate`, catalogue, `LiveState`. Source unique du contrat, exportée en TS par `ts-rs` vers `ui/src/lib/generated/`.
- `crates/module-api/` (`hubergamof-module-api`, MPL-2.0, **aucune dépendance GPL**) — traits `ModuleFactory`/`Module` (ne jamais bloquer, `HANDLER_BUDGET` 500 ms), `Command`, `ModuleContext`, descripteurs et réglages (`SettingField`/`SettingKind`), capacités `ModuleCapabilities`, actions du menu circulaire (`ModuleFactory::radial_actions`, `Module::on_radial_action`), événements haptiques (`ModuleFactory::haptic_events`, `Command::haptic`), structs `#[non_exhaustive]` construites par constructeurs.
- `crates/modules/` (`hubergamof-modules`) — modules intégrés : audio, pomodoro, docker, intellij, claude, notifications, teams ; `builtin_registry_with(BuiltinServices)` (`system()` ou `simulated()` pour `--demo` et **tous** les tests) ; briques communes (`background.rs`, `process.rs`, `paths.rs`, `trust.rs`), couche `desktop/`.
- `crates/mouse/` (`hubergamof-mouse`) — HID++ 2.0 des souris Logitech : trames (`frame.rs`), canal et échange (`channel.rs`), ping et lecture du nom d'un appareil (`probe.rs`, lecture seule), `hidraw.rs` (`hidapi`), `controls.rs` (cid par modèle), `link.rs` (batterie, sensibilité, molette, détournement des boutons, haptique : seule partie qui écrit), `session.rs` (`MouseSession` : applique un `MouseConfig`, suit les boutons détournés, les rend, remet les réglages d'origine ; `MouseSettings`) ; commande `hubergamof mouse-test [--model] info|haptic|dpi|wheel|buttons|events|release` (`diagnostics.rs`, `docs/mouse-hidpp.md`). Support des souris MX en cours (étapes 0 à 7, `docs/decisions.md` 2026-11-19).
- `crates/manager/` (`hubergamof`, lib `hubergamof_manager` + binaire) — `device/` (backends, matériel : un thread par appareil), `render/` (PNG/JPEG/SVG → RGBA, bande, caches), `strip/`, `config_store/` (JSON, `files.rs` écritures atomiques et privées, `mouse.rs` mouse.json + radial-menu.json, zip d'export/import, migrations), `engine/` (`Supervisor` → une tâche `DeviceEngine` par appareil ; `device_engine.rs` + `compose|gestures|surfaces|lock|strip.rs`, `Painter`, `notifications.rs`), `server/` (axum `127.0.0.1`, `security.rs`, `api.rs`, `session.rs`), `tray/`, `update/` (mises à jour : API des releases GitHub, `UpdateHandle`, installateur, `apply.rs` = logique de l'assistant root) + second binaire `hubergamof-update-helper` (root, lancé par systemd à chaque connexion au socket `/run/hubergamof-update.sock`), `mouse_hub.rs` (`MouseHub` : souris connectées, `mouse.json` suivis en `watch`, ce que chaque souris a refusé (`StatusReporter`, message `mouseStatus`), `radial-menu.json`, enregistrement), `server/mouse_feed.rs` (ce qu'un client WebSocket suit des souris), `mouse_engine.rs` (`MouseSupervisor` : cherche les souris toutes les 5 s, un thread par souris qui charge `mouse.json`, applique (et réapplique les configurations enregistrées pendant le service) et suit les boutons ; servie par défaut, `--no-mice` pour couper), `haptics.rs` (`HapticOutput` : où vont les événements haptiques des modules, `HapticRelay` branché plus tard ; `MouseHub` le met en œuvre : par souris, niveau et réglage de l'événement, file de huit, jouée par le fil de la souris), `mouse_actions.rs` (`ButtonActions` : périphérique virtuel partagé, fermé si verrouillé ; `ButtonDispatcher` : clic, double clic, raccourci, bascule de molette), `input/` (touches et boutons envoyés pour un bouton de souris : `Injector`, liste fermée des touches, `UinputInjector` via `input-linux`), `radial/` (menu circulaire sans dessin ni périphérique : `geometry.rs` pétales, vide, bord d'écran ; `interaction.rs` machine d'état clic/maintien/Échap, temps fourni par l'appelant ; `paint.rs` dessin en image par le `TextRenderer`, icônes composées après coup car resvg est sans images raster ; `looks.rs` `Looks` : nom et icône de chaque module (registre + `Renderer`, 96 px), donnés au contrôleur ; `controller.rs` entrées affichées + événements de la fenêtre → effets ; `window.rs` ce que les fenêtres partagent (contenu, trait `Surface`, copie dans un tampon, durée max) ; `overlay.rs` fenêtre Wayland layer-shell sur son fil (`smithay-client-toolkit` sans `xkbcommon` : Échap lu par son code de touche) ; `overlay_x11.rs` fenêtre X11 plein écran ARGB (`x11rb`, Xorg ou XWayland : GNOME) ; `backend.rs` choisit layer-shell puis X11 ; `preview.rs` commande `hubergamof radial-preview` ; `actions.rs` `ModuleActions` : l'entrée choisie devient `EngineHandle::radial_action` sur le premier deck qui a le module ; `service.rs` le menu tel que le gestionnaire le fait tourner : un fil qui porte contrôleur + fenêtre (trait `Surface`, faux dans les tests) et suit boutons, fenêtre, configuration et verrouillage ; `RadialHandle` = ce que `ButtonDispatcher` appelle pour le bouton du menu), `launcher.rs`, `screen_lock.rs`, `file_chooser.rs`.
- `ui/` — SvelteKit statique (`ui/build/`, fallback `200.html`) ; client WebSocket `lib/connection/`, état `lib/deck/` (`DeckState` porte aussi `mice: MouseState` ; `EditQueue` = file d'enregistrements sérialisés commune au deck, à une souris et au menu ; `config-edits.ts`, `mouse-edits.ts`, `shortcut.ts`, `settings-form.ts`), composants `lib/components/` (`Switch` et `SegmentedControl` : interrupteur et choix segmenté communs ; `DeckEditor` orchestre le Stream Deck, `MouseEditor` les souris : en-tête, onglets Touches (`MouseControls` : `MouseDiagram` dessine `mouse-art.ts`, liste, `ButtonActionEditor`), Pointeur et molette, Haptique ; `RadialMenuPage` = menu circulaire (aperçu `RadialPreview`, bouton déclencheur de la souris, `RadialMenuEditor`) ; `SaveTracker` + `SaveStatusLine` = état d'enregistrement commun ; la page bascule par un groupe de boutons `aria-pressed` « Stream Deck / Souris / Menu circulaire »), design « thé et bergamote » (`routes/layout.css`, jetons sémantiques sans variante `dark:`) ; e2e `ui/e2e/` (fixture `demo-manager.ts`, axe WCAG 2.2 AA).
- `packaging/` — unité systemd utilisateur, règle udev, lanceur de bureau, `install.sh`, `package/` (`.deb`, pacman) ; release : `docs/release.md`.

## Modèle de domaine
- `Device` — serial, modèle, capacités (grille de touches, taille image, molettes, bande tactile).
- `DeviceConfig` — luminosité, accueil (instances de modules ordonnées), `DialAssignment` par molette, `StripConfig`, thème (`DeviceTheme`), réglages des modules, images/libellés remplacés, `lock_sleep_seconds` ; schéma 3 (1 et 2 migrés).
- `MouseConfig` (par souris, `config/devices/<serial>/mouse.json`, schéma 1) — liaisons `MouseControlId` → `ButtonAction` (aucune, clic, double clic, bascule de molette, menu circulaire, action de module, raccourci ; un contrôle sans entrée reste natif), pointeur, molette, molette du pouce, haptique (`HapticSettings`) ; `MouseModel::capabilities()` donne les contrôles de chaque modèle (clic gauche/droit verrouillés) ; `RadialMenuConfig` (globale, `config/radial-menu.json`, schéma 1) — entrées par module, action principale au clic (ou fermer), durée du maintien. Types dans `crates/protocol/src/mouse/` et `radial.rs` ; les identifiants HID++ des boutons ne sont pas dans le contrat (ils iront dans `crates/mouse`).
- `ModuleInstance` — type de module + id + réglages ; fournit touche, pages, fonctions de molette, widget et menu de bande.
- `Page` → liste de `Key` (image, libellé, `Action` | `OpenPage`) ; `NavState` par appareil (pile de pages, ligne visible).

## Décisions techniques
Journal complet et daté : `docs/decisions.md` (y ajouter toute décision) ; sécurité : `docs/security.md`. Structurantes :
- Rust (profil release `lto = "thin"` + `codegen-units = 1` : binaire −18 % pour le même temps de build) ; un seul binaire servant UI statique + WebSocket sur `127.0.0.1:7878` ; `Origin`/`Host` stricts sans jeton de session (CSRF WebSocket, DNS rebinding).
- Appuis simulés refusés hors `--demo` (`SimulatedInput`) : sur matériel réel, les actions ne viennent que de l'appareil, l'UI configure et affiche.
- `DeckBackend` virtuel (`--demo`) pour tests, dev UI, Playwright, Lighthouse. Un thread par appareil (pas de feature `async` d'`elgato-streamdeck`).
- Pas d'API REST publique (routes HTTP internes) ; export en zip (`config.json` + `images/`).
- Bande tactile globale dessinée par l'hôte, présente dès que l'appareil la déclare (`capabilities.touchStrip`, règle partagée hôte et UI : une bande sans molette, type Stream Deck Neo, est dessinée sans rappels) ; molettes = fonctions (hôte : navigation, contextuelle, rien ; modules : volume, pomodoro…) ; thème Clair/Sombre par appareil ; plus aucun cadre sur les touches (sauf curseur/pastille de la molette contextuelle).
- Placement : un module n'est assignable à une touche que s'il le déclare (`key`), multi-instances seulement si `multiple_instances` (`placement.rs`).
- Verrouillage de l'ordinateur : deck inerte sauf `usable_when_locked` ; exceptions par page (`lock_safe_pages`, Teams) ; limites dans `docs/security.md`.
- Teams : extension de navigateur + hôte natif `hubergamof native-host` → socket Unix privé ; aucune connexion tierce.
- Mises à jour : seule connexion sortante = `api.github.com` (dépôt public, sans jeton, 24 h, `--no-update-check` pour la couper) ; installation automatique des paquets seulement : assistant root `hubergamof-update-helper` (unité système activée par socket, sans `pkexec` ni polkit : reçoit trois trames, jamais un chemin) qui vérifie la signature Ed25519 de `SHA256SUMS` (clé publique `crates/manager/assets/release-signing.pub`, privée = secret `RELEASE_SIGNING_KEY`, release signée par `sign-checksums.sh`), l'empreinte et que la version est plus récente ; `--demo` ne touche jamais le réseau ni le système. Voir `docs/release.md` § Signature, `docs/security.md`.
- Extensibilité : trait `Module` ; chargement externe (WASM/sous-processus) plus tard, jamais de `.so`. Texte rendu via `usvg`/`resvg` (pas `ab_glyph`, RUSTSEC-2026-0192).
- Icônes de modules : logos officiels, usage personnel non distribué ; à revoir si le dépôt devient public.

## Pièges et conventions spécifiques
Détail (Docker, notifications animées, Painter, tests du moteur, zbus, Teams…) : `docs/architecture.md` § « Pièges ». Règles par techno : `.claude/rules/`.
- `ui/src/lib/generated/` est généré : `make gen-types`, jamais à la main ; `ts-rs` recopie la rustdoc des types du contrat (toute retouche impose `make gen-types`). `.cargo/config.toml` fixe `TS_RS_EXPORT_DIR` et `TS_RS_LARGE_INT=number`.
- SvelteKit 3 : pas d'alias `$lib`, imports `#lib/*`.
- Le gestionnaire ne bloque jamais sa boucle d'appareil : E/S longue → `TaskSpawner` + `EventSender`/`RedrawHandle` ; `Renderer` et `ConfigStore` sont synchrones : `spawn_blocking`.
- Toute requête vers le gestionnaire porte un `Host`/`Origin` autorisé (curl : `-H 'Host: 127.0.0.1:<port>'`) ; `make dev` passe `--dev-origin`.
- Applications lancées par le gestionnaire : via `desktop::Launch` (scope systemd), sinon elles meurent avec le service. Wayland : mise au premier plan par script KWin sous KDE ; sous GNOME la fenêtre peut seulement clignoter.
- Images : `ConfigStore` adressé par contenu (ne jamais réécrire en place) ; `ImageSource::Bytes` réservé aux octets à longue durée de vie, sinon `Renderer::check_image` ; les appareils n'acceptent que JPEG (BMP Original v1/Mini) ; SVG strict (`ImageKind::sniff`).
- Souris : le détournement d'un bouton est temporaire et le laisse mort tant qu'on ne le rend pas (`MouseLink::release_buttons` au démarrage, à l'arrêt, à la reconnexion) ; Solaar, logiops et Options+ se disputent les mêmes détournements : ne pas les lancer en même temps (`--no-mice` pour garder Solaar) ; les écritures HID++ sont validées sur le matériel ; la force haptique est persistante (survit à l'extinction, `docs/mouse-hidpp.md`).
- Fenêtre X11 du menu : testée avec `Xvfb :99 -screen 0 1280x720x24 +extension Composite -fbdir <dir>` + `openbox` + `picom --backend xrender` (xcompmgr ne compose pas ici) + `xdotool` ; le rendu se lit dans `<dir>/Xvfb_screen0` (`convert xwd:…`), pas par `import -window root` qui ne voit pas la composition ; la transparence se prouve avec une fenêtre `display` rouge dessous (pixels rouges dans les écarts du menu).
- Fenêtre du menu circulaire : testée sans écran physique avec `sway` headless (`WLR_BACKENDS=headless`) + `grim` ; un sway headless n'a aucun périphérique de saisie : pour le pointeur, un petit client `zwlr_virtual_pointer` qui reste vivant (`wlrctl` crée un pointeur éphémère, trop court). Wayland ne donne la position du pointeur qu'au-dessus d'une fenêtre : la surface couvre la sortie et le menu s'ouvre à la première position reçue ; le pointeur n'est pas recentré au bord (aucun protocole).
- Une seule application peut tenir l'appareil : StreamController entre en conflit. Ne pas activer de feature de backend `hidapi` (deux backends font paniquer le build).
- Teams : l'extension suit la version du workspace (`set-version.sh` fixe `manifest.json` ; en local `0.1.0` des deux côtés).
- Session cloud : `apt-get install libudev-dev` (après `apt-get update`), Node 24 requis par pnpm 12 (tarball nodejs.org + `corepack enable --install-directory`), Playwright 1.65 attend `chromium*-1254` et `chromium_headless_shell-1254` (liens symboliques vers la version préinstallée de `/opt/pw-browsers`).
- Tests navigateur : `toHaveTextContent(/…/)` compare en entier → `getByText(/…/)` ; `<img alt="">` par `document.querySelectorAll`. Tests de panique des modules : message sur stderr attendu.
- Requêtes vers des API publiques : User-Agent générique, jamais d'information personnelle.

## État du projet
- En cours : support des souris Logitech MX (étapes 0 (crate `mouse`, relevés dans `docs/mouse-hidpp.md`) et 1 (domaine et contrat : `MouseConfig`, `RadialMenuConfig`, `HapticWaveform`) faites, ainsi que persistance, moteur, boutons, noyau, dessin et fenêtre du menu circulaire ; branchement du menu aux boutons ; actions des modules (pomodoro, audio, teams, intellij) ; événements haptiques des modules (pomodoro, teams, audio) et du menu lui-même ; interface de configuration faite (catalogue avec actions et événements, `MouseState`, page souris, `--demo` liste une MX Master 4 et une MX Anywhere 3S simulées) ; validée sur la vraie souris ; souris servies par défaut) ; reste : menu et vibrations de Teams non testés sur matériel ; l'écriture haptique est persistante (relevé du 2026-11-21)). Bureau visé d'abord : KDE/Wayland (Manjaro), GNOME (Ubuntu 24.04) en repli.
- Fait : tout le socle (protocole, moteur, rendu, config, serveur web/WebSocket, tray, UI de configuration, images, export/import), bande tactile globale, molette contextuelle, verrouillage, lanceur de bureau, modules audio, pomodoro, docker, intellij, claude, notifications, teams (pont, extension, installateur, rappels de réunions, appel entrant côté module) ; revues des spécialistes faites et corrigées (audit complet du 2026-11-18 : 0 bloquant). Historique : `docs/decisions.md`.
- À valider sur le vrai Stream Deck+ : latence des molettes avec écritures de bande, appui long de la molette contextuelle, lisibilité du cadre sur les touches, extinction au verrouillage (vrai KDE/GNOME), lanceur de bureau, extension Teams dans Brave (sélecteurs non validés ; reste la détection de la sonnerie et les noms des non lus par D-Bus).
- À valider sur poste réel : mises à jour automatiques (activation du socket par systemd, `apt-get`/`pacman` depuis l'assistant, redémarrage du service) après la première release signée.
- Reportés : déduplication des images du `liveState` (étape miroir), écritures d'une frame en lot, hotplug udev, registre des modules vers le gestionnaire, CodeQL/Semgrep (dépôt public), `hubergamof open` : arrêter l'attente quand l'unité est `failed`.
- Ouvert : option `--bind` pour le conteneur ? Non par défaut (écoute `127.0.0.1` seulement) ; à rouvrir si un usage réel l'exige, avec jeton de session.

## Licence
GPL-3.0-or-later ; `crates/module-api` sous MPL-2.0. Toute dépendance doit être compatible (vérifié par `cargo-deny`).

## Règles de code
- Clean code systématique : nommage explicite, fonctions courtes à responsabilité unique, SOLID, DRY/KISS/YAGNI, pas de nombres magiques, erreurs gérées explicitement, pas de code mort ni commenté, commentaires pour le « pourquoi ».
- Bonnes pratiques officielles de chaque techno ; rester idiomatique (Rust : `Result` + `thiserror`/`anyhow`, pas d'`unwrap` hors tests ; Svelte 5 : runes).
- En cas de conflit, la bonne pratique de la techno l'emporte sur la règle générique.
- Tout nouveau code est accompagné de tests.

## Git
- Conventional Commits (anglais, impératif) ; branches `feat/…`, `fix/…`, `chore/…` depuis `main`.
- Noms de branche parlants en kebab-case anglais : `claude/<type>/<sujet>` (ex. `claude/feat/docker-compose-file-names-icons`), sujet d'au moins deux mots ; sans préfixe `claude/` : `<type>/<sujet>` ; jamais `fix/bug`, `feat/update`, `wip`, `test` ni identifiant aléatoire. Préfixe imposé par l'outil (ex. `claude/`) conservé, suite parlante. Renommer si le périmètre change, avant push/PR.
- **Renommage obligatoire** : la branche générée par l'outil (`claude/<adjectif>-<nom>-<hasard>`) est renommée (`git branch -m`) **avant le premier commit** de la tâche, puis poussée sous son nouveau nom (c'est une autorisation explicite d'Adrien, qui prime sur « pousser sur la branche désignée »). Appliqué par `make check-branch` (dans `make check`) et par une étape du job `lint` de la CI (`.github/scripts/check-branch-name.sh`).
- SemVer ; `CHANGELOG.md` (Keep a Changelog) à jour pour tout changement notable.
- Avant chaque commit : nom de branche conforme (`make check-branch`), lint + tests au vert.

## Maintenance de ce fichier (obligatoire)
- Lire ce fichier avant d'explorer le code ; s'y fier plutôt que tout re-parcourir.
- À la fin de chaque tâche, AVANT le commit, le mettre à jour si la tâche a changé : stack, versions, dépendance majeure, commandes, CI, API, équipe d'agents, architecture ou emplacement de fichiers clés, modèle de domaine, décision, piège découvert, état du projet. Même commit que le changement.
- Rester concis (< ~200 lignes) : condenser, déplacer le détail dans `docs/` ou `.claude/rules/`, supprimer l'obsolète.
