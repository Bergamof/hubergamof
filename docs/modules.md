# Modules intégrés (`crates/modules/`)

Détail de chaque module et des briques communes ; `CLAUDE.md` n'en garde que le résumé. Les mécanismes du bureau (KWin, notifications, lancement) sont dans `integrations-desktop.md`.

## Registre et services
- `builtin_registry_with(BuiltinServices)` enregistre les fabriques et fournit la disposition par défaut (`DefaultLayout`).
- Services injectés : `audio`, `docker`, `notifier` (notifications de bureau du pomodoro), `pomodoro_clock` (temps monotone `CLOCK_BOOTTIME`, veille comprise), `desktop`, `intellij`. L'heure murale du module clock n'est pas injectée (toujours celle du système).
- `BuiltinServices::system()` : vrai `pactl`, Docker, bus de session `session_bus::SessionBus` partagé, bureau ; tout se connecte au premier usage.
- `BuiltinServices::simulated()` : `--demo` et **tous** les tests (jamais de vrai `pactl`/Docker/D-Bus/KWin).
- Chaque nouveau module ajoute son service à `BuiltinServices`.

## Briques communes
- `background.rs` : `ReadPacer` (relectures regroupées et espacées), `Backoff`, `Listener`, `WorkerControl`/`spawn_worker` (worker partagé, démarré au 1er abonné, arrêté au dernier), `spawn_action` (action détachée + `catch_panic`), `forward_states`.
- `process.rs` : commande bornée (délai, stdout borné, fin de stderr).
- `paths.rs` : `user_home()` (HOME absolu ou rien) et `~/` des réglages via `expand_home`.
- `trust.rs` : fichier que personne d'autre ne peut modifier (propriétaire, droits des ancêtres, liens résolus) ; partagé par compose et IntelliJ.
- `module-api` fournit aussi `Command::alert()`, `Feedback::of(ok)`, `KeyContent::with_icon`.
- Icônes : SVG 128×128 (pictogramme coloré sur fond noir) dans `crates/modules/assets/icons/`, embarqués par `icons::asset!("nom")` ; une constante dans le module quand il n'en a qu'une, sous-module `icons.rs` au-delà. Logos officiels Docker, IntelliJ et Claude (glyphes Simple Icons, CC0 ; marques à leurs titulaires, usage personnel — à revoir si le dépôt devient public) ; le reste est dessiné. Docker : baleine (module), cube par conteneur, couches par projet compose, chacun en actif/arrêté/attention avec badge ; audio : badge tick vert sur l'appareil sélectionné (la couleur ne porte jamais seule l'état). Provenance : `assets/icons/README.md`.

## Modules
- **clock** (jiff) : cadran « Éphéméride » (`ZoneContent::Clock`, mise en page dans `render/clock.rs`) — feuille de calendrier (jour de la semaine, quantième, mois), heure en Noto Sans Light, `semaine N` (ISO 8601) ; avec `showSeconds` : secondes en exposant teal contre les minutes et filet de 60 s à repères de 15 s (l'heure passe alors de 0,40 à 0,34 × hauteur) ; sans `showDate` : ni feuille ni semaine, l'heure prend toute la largeur. Une image par seconde (cache en probation). `liveState` : `[heure(:ss), « mer. 5 nov. · semaine 45 »]`. **navigation**.
- **audio** : `hub.rs`, un worker partagé par tous les appareils, un seul `pactl subscribe`, changements fusionnés et sérialisés, relectures ciblées et regroupées ; `pactl.rs` délai 5 s et sorties bornées ; `simulated.rs`.
- **pomodoro** : minuteur pur `timer.rs` sur `time::Clock` ; notifications de bureau `notifications.rs`.
- **docker** :
  - `hub.rs` : un flux d'événements partagé, listes regroupées, actions en tâches détachées de l'instance ;
  - `engine.rs` : bollard sur socket `unix://` uniquement ;
  - `compose.rs` : `docker --host=<même socket> compose`, sans `DOCKER_CONTEXT`, fichiers/dossier/`.env` contrôlés par `trust.rs` ;
  - `model.rs` : projets par labels + `composeFiles` déclarés prioritaires ; labels divergents → pas de compose.

## Couche bureau (`desktop/`, socle d'intellij et claude)
- `DesktopBackend` (`SystemDesktop` / `SimulatedDesktop`) derrière un `DesktopHub` unique (un seul suivi KWin), créé par `builtin_registry_with` avec les `DesktopApp` des modules.
- `DesktopApp` : classe de fenêtre, `ExecutableMatch`, `desktop-entry`, `NotificationRelay` (navigateur qui notifie pour l'appli) → `AppState` (`running`, `windows`) + `AppNotification` (expéditeur seul, textes jamais copiés).
- `followed_app.rs` : `FollowedApp`, ce qu'un module garde de son appli (état, `AppAttention`, activation, lancement, issues des actions en `AppEvent`) ; vérifie que le hub suit l'appli.
- `attention.rs` : `AppAttention` (`with_windows` / `notifications_only`) : fenêtre qui demande l'attention, ou notification reçue hors premier plan → attention jusqu'à l'appui ou au premier plan.
- `hub.rs` : lancements par `DesktopSubscription::launch(app, …)` → `LaunchStart`, un à la fois par appli toutes instances confondues (`launch_gate.rs`).
- `kwin.rs` (script filtré + objet `Report` réservé à KWin, activation ponctuelle), `script_file.rs` (écriture par descripteur), `monitor.rs` (`BecomeMonitor` au premier `notifications()` seulement, attribution par l'exécutable de l'expéditeur, tâche dédiée), `processes.rs` (`/proc/*/exe`, toutes les 2 s, 6 s après 5 sondages sans changement, de nouveau 2 s dès qu'une fenêtre apparaît ou disparaît ou qu'un lancement réussit), `launcher.rs` (`systemd-run --user --scope`, depuis `$HOME`).

## IntelliJ (`intellij/`)
- `catalog.rs` : `ProjectCatalog` injecté (`SystemCatalog` / `SimulatedCatalog`) ; fenêtre ↔ projet par le titre « projet – fichier ».
- `toolbox.rs` : installation Toolbox ou `/opt` (product-info le plus récent), lanceur `bin/idea` seul accepté, vérifié par `trust.rs` et revérifié à chaque lecture.
- `recent.rs` : `recentProjects.xml` via roxmltree sans DTD.
- `watch.rs` : catalogue sondé toutes les 5 s par un seul worker pour toutes les instances (délai 10 s, jamais deux lectures à la fois), mis en cache par (chemin, mtime, taille).
- Touche d'accueil en attention sur fenêtre ou notification (`AppAttention::with_windows`) ; page des projets récents.

## Claude (`claude/`)
- Une touche : fenêtre → activation de la première ; fermé ou dans la barre système → `gtk-launch com.anthropic.Claude`.
- Attention sur notification seulement (`AppAttention::notifications_only`), de Claude Desktop ou de claude.ai dans Brave (`NotificationRelay` : exécutable de Brave + hint `x-kde-origin-name`) ; `demandsAttention` ignoré, Claude clignote dès qu'il perd le focus.
