# Bergadeck

Gestionnaire de Stream Deck (cible : Stream Deck+, toute la gamme par conception) pour Linux, à base de modules, avec une interface de configuration web locale synchronisée en temps réel.

## Stack
- Gestionnaire : Rust 1.99.1 (édition 2024), tokio, axum (HTTP + WebSocket), `elgato-streamdeck` (+ `hidapi`), tray SNI/dbusmenu maison sur `zbus` 6, `zbus` (D-Bus), `bollard` (Docker), `image` + `resvg`, `rust-embed`, `ts-rs`.
- Interface : SvelteKit 3.1 (`adapter-static`), Svelte 5.57, Vite 8, Tailwind 4.3, TypeScript 6 (TS 7 exclu par svelte-check/typescript-eslint), pnpm 12.16 (Corepack), Node 24 LTS ; ESLint 10 + Prettier 3, Vitest 5, Playwright 1.64.
- Persistance : fichiers JSON par appareil (`config/devices/<serial>/`), pas de base de données.
- Déploiement : un seul binaire `bergadeck` (UI embarquée), service utilisateur systemd + règle udev.

## Environnement
- Versions épinglées dans : `rust-toolchain.toml`, `.nvmrc`, `ui/package.json` (`packageManager`, `engines`) — source de vérité, lue aussi par la CI.
- Activation (manuelle et auto) : voir la section dédiée du `README.md`.
- Toute montée de version d'outil se fait dans ces fichiers ET dans le `Dockerfile` (et le README si la démarche change), dans le même commit.
- Prérequis système : compilateur C + `pkg-config` + `libudev-dev` (hidapi compilé statiquement par `elgato-streamdeck`, `libhidapi` système inutile), `pactl`/`wpctl` (PipeWire), Docker (utilisateur dans le groupe `docker`), règle udev `packaging/udev/70-bergadeck.rules`.

## Commandes (Makefile racine)
- Tout vérifier : `make check` (lint + tests + contrat TS à jour)
- Lint/format : `make lint` / `make fmt`
- Tests : `make test` (Rust : `cargo test -p <crate> <nom>` ; UI : `pnpm -C ui test -- <fichier>`) ; e2e : `make test-e2e`
- Types TS du contrat : `make gen-types` (ne doit produire aucun diff en CI)
- Build : `make build` (UI puis binaire release qui l'embarque)
- Lancement local : `make dev` (gestionnaire en `--demo` + `pnpm dev` avec proxy) ; matériel réel : `make run`
- Aussi : `make check-versions` (Dockerfile ↔ `rust-toolchain.toml`/`.nvmrc`), `make deny`, `make lighthouse`, sous-cibles `lint-rust|lint-ui|test-rust|test-ui|ui-build` (appelées telles quelles par la CI).
- pnpm : `PNPM ?= corepack pnpm` lancé depuis `ui/` (Corepack lit `packageManager` du dossier courant) ; `make PNPM=pnpm …` en CI.

## CI
- `.github/workflows/ci.yml` : sur chaque PR → `lint` (+ actionlint, shellcheck), `test`, `contract`, `e2e`, `build`, `docker` (+ Trivy image), `security` (cargo-deny, pnpm audit, gitleaks, Trivy fs), `lighthouse` → job `report` (`if: always()`) qui commente la PR. Actions composites : `.github/actions/setup-rust|setup-ui`.
- Actions tierces épinglées par SHA ; Dependabot ne suit pas `LHCI_VERSION`, gitleaks ni actionlint (à la main).
- Toute nouvelle vérification est ajoutée à la CI ET aux `needs` du job `report`.
- Pas encore de dépôt distant : CI non exécutée (« terminé localement, CI non vérifiée »).

## Équipe d'agents (`.claude/agents/`)
| Agent | Rôle | Écrit | Périmètre |
|---|---|---|---|
| Chef Bouillon (session principale) | plan, briefs, routage, arbitrage | oui | `README.md`, `CHANGELOG.md`, `LICENSE`, `crates/module-api/LICENSE`, `CLAUDE.md`, `.gitignore`, `.editorconfig`, `.claude/**` |
| `rouille-et-paillettes` | dev Rust | oui | `crates/**` (hors LICENSE), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`, `.cargo/config.toml`, `ui/src/lib/generated/**` (généré), `config/example/**` |
| `svelte-et-sauvage` | dev SvelteKit/TS/Tailwind | oui | `ui/**` (hors `ui/src/lib/generated/**`), `.nvmrc` |
| `tuyauteur-fou` | CI/CD, packaging | oui | `.github/**`, `Dockerfile`, `.dockerignore`, `Makefile`, `packaging/**`, `lighthouserc.json` |
| `gardien-du-phare` | UX & accessibilité (Lighthouse ≥ 90 A11y & Bonnes pratiques) | non | rapports |
| `turbo-escargot` | performances | non | rapports |
| `cerbere-le-paranoiaque` | sécurité & CVE | non | rapports |
| `maniaque-du-plumeau` | architecture & clean code | non | rapports |
| `capitaine-assertion` | stratégie de tests | non | rapports |

## Méthode de travail
- Tâche non triviale : d'abord un plan d'action (choix techniques et fonctionnels justifiés, étapes, livrables, agents mobilisés, risques), puis attendre le « go » d'Adrien avant de coder. Écart significatif en cours de route → resoumettre.
- Seuls les agents de dev (sur leur techno) et `tuyauteur-fou` (pipeline) écrivent du code ; le chef écrit ce qui n'est à personne. Les spécialistes rendent des rapports (sévérité, localisation, recommandation, agent propriétaire) que les propriétaires doivent suivre.
- Boucle : dev → rapports des spécialistes concernés → corrections par les propriétaires → nouvelle passe, jusqu'à zéro bloquant/majeur. Désaccord : le chef arbitre ; enjeu significatif → Adrien.
- Réutiliser les agents existants ; nouvelle techno → nouvel agent de dev (nom rigolo) ; jamais de suppression d'agent sans accord d'Adrien. Piège appris → enrichir la définition de l'agent.
- Ne solliciter que les spécialistes concernés par la modification.

## Définition de « done » (sauf indication contraire d'Adrien)
- Terminé seulement quand toutes les CI déclenchées par les modifications sont vertes sur le dernier commit poussé. Des tests locaux verts ne suffisent pas.
- Après chaque push / ouverture de PR : suivre les runs jusqu'à la fin (outils de suivi de la session, ou `gh` / API GitHub sur le SHA poussé).
- Échec : lire les logs, identifier la cause racine, faire corriger par le propriétaire du fichier (code → agent de la techno ; pipeline → `tuyauteur-fou`), repousser, re-surveiller.
- Interdit pour obtenir le vert, sauf accord explicite d'Adrien : skipper/désactiver un test, assouplir le lint, `continue-on-error`, supprimer un job, baisser un seuil (couverture, Lighthouse, sévérité CVE).
- Échec non lié (flaky, infra, service externe) : relancer une fois, puis signaler avec le diagnostic. Après ~3 échecs sur le même problème : s'arrêter et exposer diagnostic et pistes.
- CI inaccessible : annoncer « terminé localement, CI non vérifiée », jamais « done ».
- CI verte : annoncer la fin avec le lien du run, puis arrêter de surveiller la PR (pas d'attente des commentaires/reviews, sauf demande).

## Architecture
- `crates/protocol/` (`bergadeck-protocol`) — messages WebSocket (`ClientMessage`/`ServerMessage`, taggés `type`, camelCase), configuration + `DeviceConfig::validate`, catalogue (`ModuleDescriptor`/`SettingField` → formulaires UI génériques), état en direct (`LiveState`, images en data URL). Source unique du contrat, exporté en TS par `ts-rs` vers `ui/src/lib/generated/` (+ `index.ts` généré par le test `export_bindings_index`).
- `crates/module-api/` (`bergadeck-module-api`, MPL-2.0) — `ModuleFactory`/`Module` (touche, pages, molette, zone, `update_settings` obligatoire), `Command` (`OpenPage`, `Back`, `GoHome`, `ScrollRows`, `Redraw`), `ModuleContext` (`RedrawHandle`, `TaskSpawner`), catalogue (`ModuleDescriptor`, `SettingField`…) et ids, ré-exportés par `protocol`. **Ne dépend d'aucun crate GPL.**
- `crates/modules/` (`bergadeck-modules`) — `ModuleRegistry`/`builtin_registry()` ; faits : clock (jiff), navigation ; à venir : audio, pomodoro, docker, intellij, claude (placeholders « X indisponible » en attendant).
- `crates/manager/` (`bergadeck` : lib `bergadeck_manager` + binaire) — `device/` (`DeckBackend`, `InputEvent`, `DeviceDiscovery`, `StripLayout`, `virtual_deck`, `hardware/` : un thread par appareil + sondage hotplug 1 s), `render/` (`Renderer` : PNG/JPEG/SVG → RGBA, libellé, zones de bande, data URL, cache LRU ; police Noto Sans OFL dans `assets/fonts/`), `config_store/` (`ConfigStore` : défaut, invalide renommé, révision, écriture atomique, images). `engine/` : `Supervisor` → une tâche `DeviceEngine` par appareil (pages, débordement en lignes, pile de navigation, rendu incrémental, `apply_config` qui conserve l'état des instances) ; `EngineHandle` (`press_key`, `rotate_dial`, `press_dial`, `apply_config`, `watch` de config et de `LiveSnapshot` → `to_live_state()` paresseux). `server/` : axum sur `127.0.0.1` uniquement (`security.rs` : `Host`/`Origin` + `--dev-origin`, CSP avec hash du script SvelteKit ; `assets.rs` : UI `rust-embed`, `_app/immutable` en cache long, fallback `200.html` ; `session.rs` : WebSocket, `liveState` ≤ 10/s, messages ≤ 1 Mio). `tray/` : StatusNotifierItem + dbusmenu sur zbus 6, ré-enregistrement si le watcher revient, clic gauche = ouvrir l'UI (`ITEM_IS_MENU = false`).
- `ui/` — SvelteKit statique (sortie `ui/build/` : `index.html` prérendu + fallback `200.html` à servir pour toute route inconnue hors assets) ; `pnpm dev`/`preview` proxifient `/ws` et `/api` vers `127.0.0.1:7878`. Client WebSocket : `ui/src/lib/connection/` (`ManagerConnection`, reconnexion avec backoff).
- `packaging/` — unité systemd utilisateur, règle udev, script d'installation.
- Flux : appareil → `DeckBackend` → moteur de pages → module → rendu → appareil ; toute modification d'état est diffusée aux clients WebSocket.

## Modèle de domaine
- `Device` — serial, modèle, capacités (grille de touches, taille image, nb molettes, taille bande tactile).
- `DeviceConfig` — luminosité, accueil (instances de modules ordonnées), affectation des molettes, réglages des modules, images/libellés remplacés.
- `ModuleInstance` — type de module + id + réglages ; fournit touche, pages, comportement de molette, zone de bande.
- `Page` → liste de `Key` (image, libellé, comportement `Action` | `OpenPage`) ; pages statiques ou générées.
- `NavState` (par appareil) — pile de pages, ligne visible.

## Décisions techniques
- 2026-10-28 — Rust pour le gestionnaire : binaire natif léger ; réticence à JS/TS hors web.
- 2026-10-28 — Un seul binaire servant UI statique + WebSocket sur `127.0.0.1:7878` : un processus, temps réel simple.
- 2026-10-28 — Contrôle strict `Origin`/`Host` (sans jeton de session, validé par Adrien, à confirmer par `cerbere-le-paranoiaque`) : les modules lancent des commandes, un site malveillant ne doit pas pouvoir les piloter (CSRF WebSocket, DNS rebinding) ; le jeton imposerait d'ouvrir l'UI via le tray.
- 2026-10-28 — Pas d'API REST publique (routes HTTP internes upload/export/import) : règles REST non appliquées.
- 2026-10-28 — `DeckBackend` virtuel (`--demo`) : tests, dev UI, Playwright et Lighthouse sans matériel.
- 2026-10-28 — Dockerfile = build reproductible + mode `--demo` (CI/Lighthouse) ; exécution réelle via systemd (USB, D-Bus, audio de session).
- 2026-10-28 — Config dans le projet (`config/devices/`, ignoré par git ; `config/example/` versionné) ; déplacement vers `~/.config` prévu plus tard.
- 2026-10-28 — Un thread par appareil matériel (sans la feature `async` d'`elgato-streamdeck`) : `HidDevice` n'est pas `Sync`, la feature utilise `block_in_place`.
- 2026-10-28 — Texte rendu via `usvg`/`resvg` (pas `ab_glyph`, qui tire `ttf-parser`, RUSTSEC-2026-0192).
- 2026-10-28 — Extensibilité : trait `Module` dès maintenant ; chargement externe (WASM ou sous-processus) plus tard, jamais de `.so`.

## Pièges et conventions spécifiques
- Wayland : mise au premier plan via script KWin sous KDE ; sous GNOME la fenêtre peut seulement clignoter (accepté). Relancer une appli déjà ouverte ne l'active pas.
- Intégrations Claude Desktop / IntelliJ / KWin / zbus 6 (mécanismes, pièges) : `docs/integrations-desktop.md` — à lire avant de toucher ces modules.
- Tray sous GNOME : nécessite l'extension AppIndicator (présente par défaut sur Ubuntu).
- `ui/src/lib/generated/` est généré : ne jamais l'éditer à la main, lancer `make gen-types` (supprime le dossier puis régénère). `cargo test --workspace` le régénère aussi. Exclu de Prettier/ESLint (non formaté par ts-rs).
- SvelteKit 3 n'a plus l'alias `$lib` : imports via sous-chemins Node `#lib/*` (ex. `import type … from '#lib/generated/index.js'`).
- `pnpm test:e2e` sert le build sur `127.0.0.1:4173` avec proxy vers 7878 : le lancer SANS gestionnaire en route (le test attend « Connexion au gestionnaire… »).
- `.cargo/config.toml` fixe `TS_RS_EXPORT_DIR` et `TS_RS_LARGE_INT=number` (u64/i64 → `number` en TS, pas `bigint`).
- `DeviceModel` reprend un pour un `elgato_streamdeck::info::Kind` (+ `virtual`) ; champs optionnels sérialisés à `null` (`T | null` en TS).
- Requêtes vers des API publiques (crates.io, npm, GitHub) : User-Agent générique, jamais d'information personnelle.
- Ne pas activer de feature de backend `hidapi` : `elgato-streamdeck` impose `linux-static-hidraw` et deux backends font paniquer le build.
- Un seul gestionnaire peut tenir l'appareil : StreamController (installé chez Adrien, autostart désactivé) entre en conflit ; son plantage a déjà détaché le pilote HID (débrancher/rebrancher).
- Les gestionnaires de module sont attendus un par un dans la boucle de l'appareil : toute E/S longue (Docker, IntelliJ, Claude, audio) passe par une tâche de fond (`TaskSpawner`) + `RedrawHandle`.
- `ts-rs` recopie la rustdoc des types du contrat dans le TS : toute retouche de ces commentaires impose `make gen-types`.
- Les touches et molettes agissent à l'appui (`KeyDown`/`DialDown`) ; relâchements et taps de bande ignorés.
- Toute requête vers le gestionnaire doit porter un `Host`/`Origin` autorisé (curl : `-H 'Host: 127.0.0.1:<port>'`). `make dev` doit passer `--dev-origin http://localhost:5173 --dev-origin http://127.0.0.1:5173`.
- UI embarquée : en debug `rust-embed` lit `ui/build/` à l'exécution ; en release elle est embarquée et `build.rs` recompile quand elle change (`make ui-build` avant `cargo build --release`).
- `Renderer` et `ConfigStore` sont synchrones/CPU : les appeler via `spawn_blocking`.
- Les appareils n'acceptent que JPEG (BMP pour Original v1/Mini) : toute image passe par le pipeline de rendu.

## État du projet
- Fait : cadrage, socle, squelette Rust + contrat `protocol`, squelette UI (client WebSocket), outillage (Makefile, CI, Docker, packaging), spikes notifications Claude + KWin (`docs/integrations-desktop.md`).
- Fait aussi : étape 5b-2 (serveur web/WebSocket, UI embarquée, tray zbus 6), validée sur le poste d'Adrien.
- Fait aussi : étape 5b-1 (module-api, moteur de pages, modules clock/navigation).
- Fait aussi : étape 5a (couche appareils, rendu, config) validée sur le vrai Stream Deck+ (touches, molettes, appuis, tap/appui long/glissement, affichage).
- Prochaines étapes : cœur du gestionnaire (étape 5), `module-api` + modules simples (6), UI (7), modules Docker/IntelliJ/Claude (8), revue des spécialistes (9).
- Ouverts : option `--bind` pour le conteneur ? Lighthouse/e2e à basculer sur `bergadeck --demo` à l'étape 5 ; CodeQL/zizmor à arbitrer avec `cerbere-le-paranoiaque` ; couverture de tests en CI.

## Licence
GPL-3.0-or-later ; `crates/module-api` sous MPL-2.0. Toute dépendance doit être compatible (vérifié par `cargo-deny`).

## Règles de code
- Clean code systématique : nommage explicite, fonctions courtes à responsabilité unique, SOLID, DRY/KISS/YAGNI, pas de nombres magiques, erreurs gérées explicitement, pas de code mort ni commenté, commentaires pour le « pourquoi ».
- Bonnes pratiques officielles de chaque techno ; rester idiomatique (Rust : `Result` + `thiserror`/`anyhow`, pas d'`unwrap` hors tests ; Svelte 5 : runes).
- En cas de conflit, la bonne pratique de la techno l'emporte sur la règle générique.
- Tout nouveau code est accompagné de tests.

## Git
- Conventional Commits (anglais, impératif) ; branches `feat/…`, `fix/…`, `chore/…` depuis `main`.
- Noms de branche parlants en kebab-case anglais ; jamais `fix/bug`, `feat/update`, `wip`, `test` ou identifiant aléatoire. Préfixe imposé par l'outil (ex. `claude/`) conservé, suite parlante.
- SemVer ; `CHANGELOG.md` (Keep a Changelog) à jour pour tout changement notable.
- Avant chaque commit : lint + tests au vert.

## Maintenance de ce fichier (obligatoire)
- Lire ce fichier avant d'explorer le code ; s'y fier plutôt que tout re-parcourir.
- À la fin de chaque tâche, AVANT le commit, le mettre à jour si la tâche a changé : stack, versions, dépendance majeure, commandes, CI, API, équipe d'agents, architecture ou emplacement de fichiers clés, modèle de domaine, décision, piège découvert, état du projet. Même commit que le changement.
- Rester concis (< ~200 lignes) : condenser, déplacer le détail dans `docs/` ou `.claude/rules/`, supprimer l'obsolète.
