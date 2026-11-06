# HuBergamof

Gestionnaire de Stream Deck (cible : Stream Deck+, toute la gamme par conception) pour Linux, à base de modules, avec une interface de configuration web locale synchronisée en temps réel.

## Stack
- Gestionnaire : Rust 1.99.1 (édition 2024), tokio, axum (HTTP + WebSocket), `elgato-streamdeck` (+ `hidapi`), tray SNI/dbusmenu maison sur `zbus` 6, `zbus` (D-Bus), `bollard` (Docker), `image` + `resvg`, `zip` (Deflate seul, export), `rust-embed`, `ts-rs`.
- Interface : SvelteKit 3.1 (`adapter-static`), Svelte 5.57, Vite 8, Tailwind 4.3, TypeScript 6 (TS 7 exclu par svelte-check/typescript-eslint), pnpm 12.16 (Corepack), Node 24 LTS ; ESLint 10 + Prettier 3, Vitest 5 (+ mode navigateur, couverture v8), Playwright 1.64.
- Persistance : fichiers JSON par appareil (`config/devices/<serial>/`), pas de base de données.
- Déploiement : un seul binaire `hubergamof` (UI embarquée), service utilisateur systemd + règle udev.

## Environnement
- Versions épinglées dans : `rust-toolchain.toml`, `.nvmrc`, `ui/package.json` (`packageManager`, `engines`) — source de vérité, lue aussi par la CI.
- Activation (manuelle et auto) : voir la section dédiée du `README.md`.
- Toute montée de version d'outil se fait dans ces fichiers ET dans le `Dockerfile` (et le README si la démarche change), dans le même commit.
- Node ≥ 24.12 (`engines`) : le Corepack des Node plus anciens (< 0.34.5) ne sait pas lancer pnpm 12 (`Cannot find module …/pnpm.cjs`, cache `~/.cache/node/corepack/v1/pnpm/<version>` incomplet à effacer) ; `make check-node` (prérequis de `ui-deps`) le détecte. Minimum lu depuis `engines` par `.github/scripts/check-node-version.sh`.
- Prérequis système : compilateur C + `pkg-config` + `libudev-dev` (hidapi compilé statiquement par `elgato-streamdeck`, `libhidapi` système inutile), `pactl`/`wpctl` (PipeWire), Docker (utilisateur dans le groupe `docker`), règle udev `packaging/udev/70-hubergamof.rules`.

## Commandes (Makefile racine)
- Tout vérifier : `make check` (lint + tests instrumentés avec planchers de couverture + doctests + contrat TS à jour)
- Lint/format : `make lint` (+ `lint-shell-exec` : aucun `sh -c`/`bash` hors tests, `.github/scripts/check-shell-exec.sh` ; `lint-rust` vérifie aussi `module-api` seul sans features) / `make fmt`
- Tests : `make test` (Rust : `cargo test -p <crate> <nom>` ; UI : `pnpm -C ui test -- <fichier>`, composants dans `*.browser.spec.ts`) ; e2e : `make test-e2e` ; couverture : `make coverage` (plancher en cliquet : mesure − 2 points, Rust dans le `Makefile`, UI dans `ui/vite.config.ts`, jamais baissé) ; vrai Docker : `make test-docker` (feature `docker-it`)
- Types TS du contrat : `make gen-types` (ne doit produire aucun diff en CI)
- Build : `make build` (UI puis binaire release qui l'embarque)
- Lancement local : `make dev` (gestionnaire en `--demo` + `pnpm dev` avec proxy) ; matériel réel : `make run`
- Aussi : `make check-node` / `make check-branch` / `make test-scripts` (scripts de `.github/scripts/` + leur auto-test), `make check-versions` (Dockerfile ↔ `rust-toolchain.toml`/`.nvmrc`), `make deny`, `make lighthouse`, `make lint-workflows` (zizmor), sous-cibles `lint-rust|lint-ui|test-rust|test-ui|coverage-rust|coverage-ui|ui-build` (appelées telles quelles par la CI).
- pnpm : `PNPM ?= corepack pnpm` lancé depuis `ui/` (Corepack lit `packageManager` du dossier courant) ; `make PNPM=pnpm …` en CI.

## CI
- `.github/workflows/ci.yml` : sur chaque PR, push sur `main` et chaque semaine → `lint` (+ actionlint, shellcheck, zizmor bloquant), `test` (tests instrumentés + couverture), `contract`, `e2e` (`failOnFlakyTests`), `build`, `docker` (+ Trivy image), `docker-it` (vrai Docker, sur PR seulement si le module est touché), `security` (cargo-deny, pnpm audit, gitleaks, Trivy fs), `lighthouse` (A11y, Bonnes pratiques, Perf ≥ 0,9) → job `report` (`if: always()`) qui commente la PR. Actions composites : `.github/actions/setup-rust|setup-ui`.
- Toutes les actions épinglées par SHA ; Dependabot ne suit pas `LHCI_VERSION`, gitleaks, actionlint (SHA-256 épinglé), zizmor ni cargo-llvm-cov (à la main, variables `env` du workflow).
- Toute nouvelle vérification est ajoutée à la CI ET aux `needs` du job `report`.
- Pas encore de dépôt distant : CI non exécutée (« terminé localement, CI non vérifiée »).

## Équipe
- Session principale (Chef Bouillon) : plan, briefs aux spécialistes, arbitrage et **toute écriture** (code Rust, UI, CI/CD, packaging, docs, `.claude/**`). Pas de sous-agent de développement (décision d'Adrien, 2026-11-01).
- Conventions et pièges par techno : `.claude/rules/rust.md`, `ui.md`, `ci-packaging.md` (chargés selon les fichiers touchés).
- Spécialistes en lecture seule (`.claude/agents/`), qui rendent des rapports :

| Agent | Modèle | Rôle |
|---|---|---|
| `gardien-du-phare` | sonnet | UX & accessibilité (Lighthouse ≥ 90 A11y & Bonnes pratiques) |
| `turbo-escargot` | sonnet | performances |
| `cerbere-le-paranoiaque` | opus | sécurité & CVE |
| `maniaque-du-plumeau` | opus | architecture & clean code |
| `capitaine-assertion` | sonnet | stratégie de tests |

## Méthode de travail
- Tâche non triviale : d'abord un plan d'action (choix techniques et fonctionnels justifiés, étapes, livrables, spécialistes mobilisés, risques), puis attendre le « go » d'Adrien avant de coder. Écart significatif en cours de route → resoumettre.
- Les spécialistes rendent des rapports (sévérité, localisation, recommandation) que la session principale doit suivre.
- Boucle : dev → rapports des spécialistes concernés → corrections → nouvelle passe, jusqu'à zéro bloquant/majeur. Désaccord avec un spécialiste : enjeu significatif → Adrien.
- Ne solliciter que les spécialistes concernés par la modification (chaque appel repart de zéro et coûte des tokens). Garder le contexte léger : filtrer les sorties de build/tests (erreurs, résumé) plutôt que les afficher en entier.
- Réutiliser les spécialistes existants ; jamais de suppression d'agent sans accord d'Adrien. Modèle fixé dans le frontmatter (`model:`) : opus là où une omission coûte cher (sécurité, architecture), sonnet pour le reste. Piège appris → l'ajouter au fichier `.claude/rules/` de la techno (ou à la définition du spécialiste concerné).

## Définition de « done » (sauf indication contraire d'Adrien)
- Terminé seulement quand toutes les CI déclenchées par les modifications sont vertes sur le dernier commit poussé. Des tests locaux verts ne suffisent pas.
- Après chaque push / ouverture de PR : suivre les runs jusqu'à la fin (outils de suivi de la session, ou `gh` / API GitHub sur le SHA poussé).
- Échec : lire les logs, identifier la cause racine, corriger, repousser, re-surveiller.
- Interdit pour obtenir le vert, sauf accord explicite d'Adrien : skipper/désactiver un test, assouplir le lint, `continue-on-error`, supprimer un job, baisser un seuil (couverture, Lighthouse, sévérité CVE).
- Échec non lié (flaky, infra, service externe) : relancer une fois, puis signaler avec le diagnostic. Après ~3 échecs sur le même problème : s'arrêter et exposer diagnostic et pistes.
- CI inaccessible : annoncer « terminé localement, CI non vérifiée », jamais « done ».
- CI verte : annoncer la fin avec le lien du run, puis arrêter de surveiller la PR (pas d'attente des commentaires/reviews, sauf demande).

## Architecture
- `crates/protocol/` (`hubergamof-protocol`) — messages WebSocket (`ClientMessage`/`ServerMessage`, taggés `type`, camelCase), configuration + `DeviceConfig::validate`, catalogue (`CatalogModule` = `ModuleDescriptor` aplati + icône PNG + réglages par défaut ; `SettingField` → formulaires UI génériques), état en direct (`LiveState`, images en data URL, envoyé seulement si `selectDevice.liveState`). Source unique du contrat, exporté en TS par `ts-rs` vers `ui/src/lib/generated/` (+ `index.ts` généré par le test `export_bindings_index`).
- `crates/module-api/` (`hubergamof-module-api`, MPL-2.0) — `ModuleFactory` (`validate_settings`, défaut par descripteur) / `Module` (touche, pages, molette, zone, `on_event`, `update_settings` obligatoire ; ne jamais bloquer, `HANDLER_BUDGET` 500 ms), `ModuleFactory::icon` (icône de l'UI), `Command` (`OpenPage`, `Back`, `GoHome`, `ScrollRows`, `Feedback` ✓/⚠), `KeyState` (normal/active/attention), `ZoneContent` (`Lines` ≤ 3, `Gauge` — `GaugeContent` titre/valeur/`GaugeLevel` en millièmes/`dimmed` —, `Clock` — `ClockFace` heure/`ClockSeconds` (borné à 59 par son constructeur)/`CalendarLeaf` (casse naturelle, capitales mises par le rendu)/légende —, `Image` ; pour une jauge `ZoneView.lines` = `[titre, valeur]`, pour une horloge `[heure(:ss), date · semaine]`), `ModuleContext` (`RedrawHandle`, `EventSender` — `false` = instance morte, arrêter —, `TaskSpawner` ; `ModuleContext::for_tests`), catalogue (`ModuleDescriptor`, `SettingField`, chemins absolus ou `~/…` : `expand_home`/`is_setting_path`, règle unique partagée avec l'UI, `ModuleDescriptor::parse_settings` qui valide les bornes puis désérialise…) et ids, ré-exportés par `protocol` ; dérives ts-rs derrière la feature `ts` ; `testing::TestContext` derrière la feature `testing` (`tokio/sync`). **Ne dépend d'aucun crate GPL.**
- `crates/modules/` (`hubergamof-modules`) — modules intégrés : clock, navigation, audio, pomodoro, docker, intellij, claude ; `builtin_registry_with(BuiltinServices)` (services injectés, dont `wall_clock` (heure murale de l'horloge, figée dans `simulated()` : tests indépendants de la minute, `--demo` en heure réelle) : `system()` réels, connectés au premier usage, ou `simulated()` pour `--demo` et **tous** les tests) + disposition par défaut. Briques communes (`background.rs`, `process.rs`, `paths.rs`, `trust.rs`) et couche `desktop/` (`DesktopHub` unique, `FollowedApp`, KWin, monitor de notifications, `/proc`, lancement en scope systemd). Détail par module : `docs/modules.md`.
- `crates/manager/` (`hubergamof` : lib `hubergamof_manager` + binaire) — `device/` (`DeckBackend`, `InputEvent`, `DeviceDiscovery`, `StripLayout`, `virtual_deck`, `hardware/` : un thread par appareil + sondage hotplug 1 s, encodages JPEG mis en cache par identité de l'`Arc<RgbaImage>` ; `device::blank`), `render/` (`Renderer` : PNG/JPEG/SVG → RGBA, libellé, zones de bande, état des touches, marques ✓/⚠, PNG encodé une fois et gardé (data URL dérivée à la demande), cache `cache::SegmentedLru` (192 images réutilisées + 32 en probation) (probation/protégé + historique : les images d'un décompte ne chassent pas les icônes, idem cache JPEG), condensats mémorisés par `(chemin, mtime, taille)` ; polices Noto Sans OFL SemiBold + Light dans `assets/fonts/` (`FontWeight`, `LineStyle`, `TextRenderer::fitted_line` mesure dans la graisse dessinée ; les SVG d'utilisateur ne voient que SemiBold ; provenance et empreintes : `assets/fonts/README.md`), `render/clock.rs` : cadran de l'horloge), `config_store/` (`ConfigStore` : défaut, invalide renommé, révision, écriture atomique, images ; `archive.rs` : zip d'export `config.json` (Deflate) + `images/` (PNG/JPEG stockés tels quels), lecture bornée — entrées autorisées seulement, nombre d'entrées lu dans l'EOCD avant analyse, doublons refusés, tailles décompressées plafonnées ; `export`, `prepare_import` (adapte via `fit_to_device`, valide, vérifie chaque image sans rien écrire) puis `write_import_images`/`remove_images` ; images inutilisées supprimées au chargement). `engine/` : `Supervisor` → une tâche `DeviceEngine` par appareil (pages, débordement en lignes, pile de navigation, rendu incrémental, appui résolu sur la frame affichée, rafales d'entrées/événements regroupées avant un seul rafraîchissement, `apply_config` validé par `validation::check_modules` avant enregistrement → `InvalidConfig`, qui conserve l'état des instances ; luminosité retentée si non appliquée). Instances isolées : panique ou dépassement du budget → placeholder ou abandon + ⚠ (`feedback.rs`, 1 s) ; `EngineHandle` (`press_key`, `rotate_dial`, `press_dial`, `apply_config`, `watch` de config et de `LiveSnapshot` → `to_live_state()` paresseux). `server/` : `api.rs` (routes HTTP internes `/api/devices/{serial}/…` : `images` POST et GET, `export`, `import` ; toute image stockée passe `Renderer::check_image` (sans cache ni mémo), le GET sert toujours le PNG rendu, jamais le fichier (pas de SVG servi) ; corps bornés, un transfert à la fois (sémaphore), erreurs `ApiError`, détails internes seulement dans les journaux ; import refusé → images ajoutées supprimées) ; `catalog.rs` (catalogue construit une fois au démarrage, icônes rendues en PNG 144 px) ; axum sur `127.0.0.1` uniquement (`security.rs` : `Host`/`Origin` + `--dev-origin`, `/api` refusé si `Sec-Fetch-Site` cross-site/same-site, CSP avec hash du script SvelteKit ; `assets.rs` : UI `rust-embed`, `_app/immutable` en cache long, variantes `.br`/`.gz` du build servies selon `Accept-Encoding` (CSP calculée sur le HTML clair), fallback `200.html` ; `session.rs` : WebSocket, `liveState` ≤ 10/s sur demande, config enregistrée renvoyée à son auteur, messages ≤ 1 Mio). `tray/` : StatusNotifierItem + dbusmenu sur zbus 6, ré-enregistrement si le watcher revient, clic gauche = ouvrir l'UI (`ITEM_IS_MENU = false`).
- `ui/` — SvelteKit statique (sortie `ui/build/` : `index.html` prérendu + fallback `200.html` à servir pour toute route inconnue hors assets) ; `pnpm dev`/`preview` proxifient `/ws` et `/api` vers `127.0.0.1:7878`. Client WebSocket : `ui/src/lib/connection/` (`ManagerConnection`, reconnexion avec backoff). `ui/src/lib/deck/` : `DeckState` (état central ; une mise à jour en vol, éditions suivantes en file ; réponse reconnue au contenu + révision suivante ; toute erreur pendant l'envoi l'abandonne ; garde 10 s ; `edit()` → `EditOutcome`), éditions pures `ConfigEdit` (`config-edits.ts`), formulaires (`settings-form.ts`, même validation que `SettingKind::check`). `ui/src/lib/api/` : `ManagerApi` (routes `/api`, erreurs en `Problem`). `ui/src/lib/components/` : `DeckEditor` orchestre (pas de miroir : icône du module sur chaque touche/molette). Utilitaires Tailwind `focus-ring`, `field`, `secondary-button`, `surface-button` (`routes/layout.css`). e2e dans `ui/e2e/` (fixture `demo-manager.ts` : un `hubergamof --demo` par test ; contrôles axe WCAG 2.2 AA des états riches, clair et sombre).
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
- 2026-10-28 — Contrôle strict `Origin`/`Host`, sans jeton de session : un site malveillant ne doit pas pouvoir piloter le gestionnaire (CSRF WebSocket, DNS rebinding).
- 2026-11-02 — Appuis simulés (`PressKey`/`RotateDial`/`PressDial`) refusés hors `--demo` (`SimulatedInput`, `ErrorCode::InputRefused`) : sur le vrai matériel, les actions ne viennent que de l'appareil, l'UI ne fait que configurer et afficher. Ferme l'élévation vers root via Docker par un processus local (Origin/Host forgeables hors navigateur) ; jeton de session écarté tant que l'UI n'a pas d'action directe.
- 2026-11-03 — UI sans affichage miroir dans un premier temps (Adrien) : chaque touche/molette montre l'icône de son module ; `liveState` gardé, sur demande.
- 2026-10-28 — Pas d'API REST publique (routes HTTP internes upload/export/import) : règles REST non appliquées.
- 2026-11-03 — Export en zip (`config.json` lisible + `images/`) plutôt qu'en JSON à images base64 (Adrien) ; import vers un autre modèle autorisé (molettes adaptées) ; images personnalisées sur les touches seulement.
- 2026-10-28 — `DeckBackend` virtuel (`--demo`) : tests, dev UI, Playwright et Lighthouse sans matériel.
- 2026-10-28 — Dockerfile = build reproductible + mode `--demo` (CI/Lighthouse) ; exécution réelle via systemd (USB, D-Bus, audio de session).
- 2026-10-28 — Config dans le projet (`config/devices/`, ignoré par git ; `config/example/` versionné) ; déplacement vers `~/.config` prévu plus tard.
- 2026-10-28 — Un thread par appareil matériel (sans la feature `async` d'`elgato-streamdeck`) : `HidDevice` n'est pas `Sync`, la feature utilise `block_in_place`.
- 2026-10-28 — Texte rendu via `usvg`/`resvg` (pas `ab_glyph`, qui tire `ttf-parser`, RUSTSEC-2026-0192).
- 2026-11-05 — Icônes par défaut des modules : logos officiels (Docker, IntelliJ, Claude) en pictogrammes colorés sur fond noir, usage personnel non distribué (Adrien) ; remplace « jamais de marque ». À revoir si le dépôt devient public. SVG dans `crates/modules/assets/icons/` (`icons::asset!`).
- 2026-11-05 — Plus aucun cadre sur les touches (Adrien) : `KeyState::Active` ne dessine rien (l'état se voit par l'icône : badges Docker, tick vert audio), `Attention` garde seulement son fond orange. Libellé de touche : une seule ligne, police 0,13 × hauteur, sous l'icône (zone d'image réduite), jamais par-dessus.
- 2026-11-05 — Horloge : cadran « Éphéméride » (feuille de calendrier + heure fine + semaine ISO, secondes en exposant et filet de 60 s), sans style au choix ni teinte du moment (Adrien : « n'apporte pas grand chose »).
- 2026-10-28 — Extensibilité : trait `Module` dès maintenant ; chargement externe (WASM ou sous-processus) plus tard, jamais de `.so`.

## Pièges et conventions spécifiques
- Wayland : mise au premier plan via script KWin sous KDE ; sous GNOME la fenêtre peut seulement clignoter (accepté). Relancer une appli déjà ouverte ne l'active pas.
- Risques acceptés et décisions de sécurité (autres comptes Unix, unité systemd, bornes) : `docs/security.md`.
- Intégrations Claude Desktop / IntelliJ / KWin / zbus 6 (mécanismes, pièges) : `docs/integrations-desktop.md` — à lire avant de toucher ces modules ; détail des modules : `docs/modules.md` (à tenir à jour avec eux).
- Tray sous GNOME : nécessite l'extension AppIndicator (présente par défaut sur Ubuntu).
- `ui/src/lib/generated/` est généré : ne jamais l'éditer à la main, lancer `make gen-types` (supprime le dossier puis régénère). `cargo test --workspace` le régénère aussi. Exclu de Prettier/ESLint (non formaté par ts-rs).
- SvelteKit 3 n'a plus l'alias `$lib` : imports via sous-chemins Node `#lib/*` (ex. `import type … from '#lib/generated/index.js'`).
- `pnpm test:e2e` : le `webServer` de Playwright construit l'UI puis la sert (il démarre avant `globalSetup`, d'où le build ici et pas dans le setup), `globalSetup` compile le gestionnaire, chaque test « demo » en lance un (port libre, config temporaire) ; le projet « sans-gestionnaire » sert le build sur `127.0.0.1:4173` avec proxy vers 7878 : rien ne doit tourner sur 7878. Lighthouse audite `hubergamof --demo` sur 7881.
- `.cargo/config.toml` fixe `TS_RS_EXPORT_DIR` et `TS_RS_LARGE_INT=number` (u64/i64 → `number` en TS, pas `bigint`).
- `DeviceModel` reprend un pour un `elgato_streamdeck::info::Kind` (+ `virtual`) ; champs optionnels sérialisés à `null` (`T | null` en TS).
- Requêtes vers des API publiques (crates.io, npm, GitHub) : User-Agent générique, jamais d'information personnelle.
- Ne pas activer de feature de backend `hidapi` : `elgato-streamdeck` impose `linux-static-hidraw` et deux backends font paniquer le build.
- Un seul gestionnaire peut tenir l'appareil : StreamController (installé chez Adrien, autostart désactivé) entre en conflit ; son plantage a déjà détaché le pilote HID (débrancher/rebrancher).
- Les gestionnaires de module sont attendus un par un dans la boucle de l'appareil : toute E/S longue (Docker, IntelliJ, Claude, audio) passe par une tâche de fond (`TaskSpawner`) + `EventSender` (changement d'état, `on_event`) ou `RedrawHandle` ; une boucle de fond s'arrête quand `send` renvoie `false`.
- `ts-rs` recopie la rustdoc des types du contrat dans le TS : toute retouche de ces commentaires impose `make gen-types`.
- Les touches et molettes agissent à l'appui (`KeyDown`/`DialDown`) ; relâchements et taps de bande ignorés.
- Toute requête vers le gestionnaire doit porter un `Host`/`Origin` autorisé (curl : `-H 'Host: 127.0.0.1:<port>'`). `make dev` doit passer `--dev-origin http://localhost:5173 --dev-origin http://127.0.0.1:5173`.
- UI embarquée : en debug `rust-embed` lit `ui/build/` à l'exécution ; en release elle est embarquée et `build.rs` recompile quand elle change (`make ui-build` avant `cargo build --release`).
- `Renderer` et `ConfigStore` sont synchrones/CPU : les appeler via `spawn_blocking`.
- Les appareils n'acceptent que JPEG (BMP pour Original v1/Mini) : toute image passe par le pipeline de rendu.
- Images du `ConfigStore` adressées par contenu : ne jamais réécrire un fichier en place (le mémo des condensats s'appuie sur `(chemin, mtime, taille)`).
- Sources `ImageSource::Bytes` : réservées aux octets à longue durée de vie (icônes) — le mémo des condensats garde un `Weak` qui retient l'allocation ; contenu ponctuel (upload, import) → `Renderer::check_image`.
- Détection SVG stricte (`ImageKind::sniff`) : document dont l'élément racine est `<svg` ; un texte qui contient `<svg` n'en est pas un (fichier compose déguisé).
- Proxies zbus vers KDE : membres en lowerCamelCase → `#[zbus(name = "…")]` (zbus dérive en PascalCase) ; builders zbus 6 sans `Result` (erreur au `build()`).
- Applications lancées par le gestionnaire : toujours via `desktop::Launch` (scope systemd), sinon elles meurent avec le service ; contraintes sur les valeurs lancées dans `docs/integrations-desktop.md`.
- Attentes des streams du bureau (`WindowChanges`/`NotificationStream::next`) : sûres à l'annulation (lues dans un `select!`) ; tout travail lent va dans une tâche dédiée.
- Les tests de panique des modules écrivent le message du panic hook sur stderr (`fragile …`) : attendu.

## État du projet
- Fait : cadrage, socle, squelette Rust + contrat `protocol`, squelette UI (client WebSocket), outillage (Makefile, CI, Docker, packaging), spikes notifications Claude + KWin (`docs/integrations-desktop.md`).
- Fait aussi : modules audio et pomodoro + corrections des revues architecture/sécurité/tests/perf (tests hermétiques, 2 `pactl` par cran au lieu de 7, mémoire stable pendant un pomodoro), validés sur le vrai Stream Deck+ (l'UI n'affiche pas encore `liveState` : étape 7).
- Fait aussi : étape 5b-2 (serveur web/WebSocket, UI embarquée, tray zbus 6), validée sur le poste d'Adrien.
- Fait aussi : étape 5b-1 (module-api, moteur de pages, modules clock/navigation) + corrections de la revue d'architecture (événements, validation, états/retours, isolation, budget) et des revues perf/tests, validées sur le vrai Stream Deck+.
- Fait aussi : étape 5a (couche appareils, rendu, config) validée sur le vrai Stream Deck+ (touches, molettes, appuis, tap/appui long/glissement, affichage).
- Fait aussi : module Docker + corrections des revues architecture/sécurité/tests, validé sur le vrai Stream Deck+ avec le vrai Docker.
- Fait aussi : étape 7a (UI de configuration sans miroir : touches, molettes, réglages, luminosité) + corrections des revues UX/tests/architecture/perf.
- Fait aussi : étape 7b (images de touche, export/import zip).
- Fait aussi : étape 8a (couche bureau `desktop/` + revues architecture/sécurité), validée en réel sur la session Plasma d'Adrien (fenêtres, activation, processus, monitor).
- Fait aussi : module IntelliJ (8b), validé sur le vrai Stream Deck+ (catalogue Toolbox, projets récents, lancement, passage à la fenêtre, attention après les tests).
- Fait aussi : notifications d'IntelliJ (spike : texte seul, envoyé en double) → touche d'accueil en attention, sans message (décision d'Adrien).
- Fait aussi : module Claude Desktop (8c) ; Claude Desktop n'envoie pas de notification → attention via claude.ai dans Brave (décision d'Adrien) ; validé sur le vrai Stream Deck+ (lancement, barre système, notification de Brave).
- Fait aussi : revue des spécialistes (9) : groupes UX, architecture, sécurité (`docs/security.md`), tests, CI (couverture, vrai Docker, composants en navigateur, zizmor, précompression) , performances (sondage `/proc` espacé au repos, cache de rendu allégé).
- Reportés (étape 9) : déduplication des images du `liveState` (étape miroir) ; écritures d'une frame envoyées en lot au thread de l'appareil (chaque écriture paie aujourd'hui le délai de lecture de 10 ms ; permettrait de l'allonger) ; hotplug par moniteur udev au lieu du sondage ; registre des modules déplacé vers le gestionnaire (avant le chargement externe) ; CodeQL/Semgrep (quand le dépôt sera public) ; m10 de la revue d'architecture.
- Ouverts : option `--bind` pour le conteneur ? Prochains composants à tester en navigateur (puis relever les planchers) : `SettingInput`, `InstanceEditor`, `DialDetails`. Risques acceptés Docker : fichiers nommés par un compose de confiance (`include`, `extends`, `env_file`) non contrôlés, fin de stderr de compose dans les journaux.

## Licence
GPL-3.0-or-later ; `crates/module-api` sous MPL-2.0. Toute dépendance doit être compatible (vérifié par `cargo-deny`).

## Règles de code
- Clean code systématique : nommage explicite, fonctions courtes à responsabilité unique, SOLID, DRY/KISS/YAGNI, pas de nombres magiques, erreurs gérées explicitement, pas de code mort ni commenté, commentaires pour le « pourquoi ».
- Bonnes pratiques officielles de chaque techno ; rester idiomatique (Rust : `Result` + `thiserror`/`anyhow`, pas d'`unwrap` hors tests ; Svelte 5 : runes).
- En cas de conflit, la bonne pratique de la techno l'emporte sur la règle générique.
- Tout nouveau code est accompagné de tests.

## Git
- Conventional Commits (anglais, impératif) ; branches `feat/…`, `fix/…`, `chore/…` depuis `main`.
- Noms de branche parlants en kebab-case anglais : `[claude/]<type>(/|-)<au moins deux mots>` (ex. `claude/fix-node-minimum-version`) ; jamais `fix/bug`, `feat/update`, `wip`, `test` ni identifiant aléatoire. Préfixe imposé par l'outil (ex. `claude/`) conservé, suite parlante.
- **Renommage obligatoire** : la branche générée par l'outil (`claude/<adjectif>-<nom>-<hasard>`) est renommée (`git branch -m`) **avant le premier commit** de la tâche, puis poussée sous son nouveau nom (c'est une autorisation explicite d'Adrien, qui prime sur « pousser sur la branche désignée »). Appliqué par `make check-branch` (dans `make check`) et par une étape du job `lint` de la CI (`.github/scripts/check-branch-name.sh`).
- SemVer ; `CHANGELOG.md` (Keep a Changelog) à jour pour tout changement notable.
- Avant chaque commit : nom de branche conforme (`make check-branch`), lint + tests au vert.

## Maintenance de ce fichier (obligatoire)
- Lire ce fichier avant d'explorer le code ; s'y fier plutôt que tout re-parcourir.
- À la fin de chaque tâche, AVANT le commit, le mettre à jour si la tâche a changé : stack, versions, dépendance majeure, commandes, CI, API, équipe d'agents, architecture ou emplacement de fichiers clés, modèle de domaine, décision, piège découvert, état du projet. Même commit que le changement.
- Rester concis (< ~200 lignes) : condenser, déplacer le détail dans `docs/` ou `.claude/rules/`, supprimer l'obsolète.
