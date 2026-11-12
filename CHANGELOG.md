# Changelog

Toutes les évolutions notables de ce projet sont consignées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet respecte le [versionnement sémantique](https://semver.org/lang/fr/).

## [Non publié]

### Ajouté

- Bande tactile globale : le gestionnaire dessine lui-même l'heure (secondes en option), la date et la semaine, la charge CPU, mémoire et swap (courbes d'historique, alerte au-delà d'un seuil), le chemin de la page, le volume et le pomodoro avec sa barre de progression, thème clair ou sombre. Sous la bande, un rappel par molette dit ce qu'elle fait (rotation, clic) et suit l'état (« Muet » puis « Réactiver »). Le menu du pomodoro s'ouvre en fenêtre flottante au-dessus de sa molette et se ferme seul après 5 s.
- Configuration : chaque molette reçoit une fonction (Rien, Navigation, Contextuelle, ou une fonction offerte par un module : « Contrôle du volume », « Contrôle du pomodoro »), par défaut Volume, Pomodoro, Contexte, Navigation. La fonction contextuelle est assignable mais n'est pas encore définie.
- Interface : aperçu de la bande (valeurs d'exemple, suit le thème et les options) ; un clic ouvre ses réglages (thème, secondes, barre de progression du pomodoro, courbes, période des mesures, seuil d'alerte).
- API des modules : fonctions de molette (`ModuleDescriptor::dial_functions`), rappels (`Module::dial_hint`), widgets de bande (`StripWidget`) et menu flottant (`StripPopup`). Contrat : schéma de configuration 2 (`DialAssignment`, `StripConfig`) ; `liveState` porte l'image de la bande entière.

- Lanceur de bureau « HuBergamof » avec son icône (paquets et `make install`) : il relance le service utilisateur quand le gestionnaire a été fermé (quitté, arrêté, abandonné par systemd après des échecs), puis ouvre l'interface. Nouvelle sous-commande `hubergamof open` (démarrage par D-Bus, sans shell).
- Release : workflow manuel `release.yml` (depuis `main` seulement, CI verte exigée sur le commit, attente d'un run en cours). Version déduite des Conventional Commits depuis le dernier tag (1.0.0 au début, forçage d'un patch possible), release GitHub avec note, `SHA256SUMS` et tag `vX.Y.Z`.
- Paquets `.deb` (Ubuntu 24.04+, Debian 12+) et pacman (Manjaro, Arch) : dépendances obligatoires dont `pactl` et un serveur de son, optionnelles pour Docker, portail de fichiers et extension de tray ; règle udev, service utilisateur activé pour tous et démarré dans les sessions graphiques ouvertes. Cibles `make package*`, job CI `package` (lintian, installation en conteneurs Ubuntu et Arch).
- API des modules : `Command::BackTo(PageId)` revient à une page du module en fermant celles qui sont ouvertes au-dessus.
- Contrat : code d'erreur `busy` (409) quand une boîte de dialogue de fichier est déjà ouverte ; le sélecteur de fichier est décrit par le réglage (`FileChooser`, `EntryNaming`, bornes `max_entries`/`max_name_chars`, textes) et la route devient `POST /api/file-chooser` (`FileChooserRequest { module, setting }`) : l'interface de liste de fichiers ne contient plus rien de propre à compose.

- Module Docker : chaque fichier compose déclaré a un nom et une icône au choix. Les réglages listent les fichiers ajoutés (icône, nom, chemin) ; on les ajoute avec le sélecteur de fichier natif du bureau (portail `xdg-desktop-portal`, qui donne le vrai chemin, ce qu'un `<input type="file">` ne fait pas), puis on les modifie ou les supprime sur place. Sans portail (ou sans bureau), le chemin se saisit. Le nom remplace celui du projet compose sur la touche et dans les titres ; l'icône remplace les couches. Les icônes sont stockées comme les images de touche (même contrôle, export/import zip, nettoyage des inutilisées) et lues par les modules via `ModuleContext::images()`.
- Contrat : réglage `SettingKind::FileEntryList` (`FileEntry` : chemin, nom, icône), route `POST /api/file-chooser/compose-file` (`FileChosen`), code d'erreur `unavailable`. Les anciens réglages `composeFiles` (liste de chemins) restent lus tels quels.
- Module Docker : les services définis par un compose mais sans conteneur (pas encore construits) apparaissent sur la page du projet, avec une icône en pointillés qui les distingue des conteneurs arrêtés ; leur page propose `up`.
- Module Docker : la page d'un conteneur compose a ses propres `up` et `down`, limités à son service ; lancer un `up` ou un `down` ramène à la page du projet, où les entrées concernées montrent l'opération en cours (`Build…`, `Pull…`, `Up…`, `Down…`).
- Module Docker : `Up --no-start` (page « Up » du projet) crée les conteneurs sans les démarrer (`docker compose up --no-start`).

### Corrigé

- Module Docker : le ✓/⚠ d'un `up`/`down` sur un service s'affiche sur son entrée (conteneur ou service sans conteneur) et non sur la touche du projet.

- `make dev` : le sélecteur de fichier compose ouvre maintenant la vraie boîte du bureau. `--demo` le remplaçait par une réponse fixe ; cette réponse est devenue l'option `--file-chooser-answer <fichier>`, que seuls les tests e2e passent.

- Sélecteur de fichier compose : le signal de réponse du portail est écouté sans dépendre du nom du service (activé à la demande sous Ubuntu), la réponse est reconnue par le chemin de la requête renvoyé par le portail, et une erreur D-Bus réelle n'est plus déguisée en « indisponible » (elle est journalisée).
- Réglages Docker : un fichier compose sans image personnalisée montre l'icône des projets compose (les couches) et non la baleine du module (`ModuleFactory::setting_item_icon`, `CatalogModule::setting_icons`).

- Image Docker : `zlib1g` est mis à jour dans l'étage d'exécution (deux CVE HIGH corrigées par Debian, absentes du digest épinglé de `debian:trixie-slim`), ce qui faisait échouer le scan Trivy de la CI.

- Bande tactile incomplète au premier démarrage ou après un rebranchement : seules les zones qui changeaient (l'horloge) s'affichaient, les écritures faites juste à la connexion étant perdues par l'appareil. Toute la bande est désormais réécrite une fois, 400 ms après le premier dessin.

- Applications lancées par le gestionnaire (IntelliJ, Claude) : elles prennent aussi leur propre groupe de processus, pour ne plus se fermer avec le gestionnaire lorsqu'un signal (Ctrl-C dans le terminal) vise le groupe de celui-ci.

- Module Audio sur Ubuntu : `pactl` n'y est pas installé par défaut (paquet `pulseaudio-utils`), contrairement à ce que disait le README. La bande affiche désormais « pactl absent » ou « pactl trop ancien » (version < 16) au lieu de « Audio indisponible », le journal indique le paquet à installer, `make install` avertit si `pactl` manque ou est trop ancien, et le README et le CLAUDE.md corrigent le prérequis (`wpctl`, inutilisé, retiré).

- Build sur une machine neuve avec un Node 24 antérieur à 24.12 : le Corepack livré ne sait pas lancer pnpm 12 (`Cannot find module …/pnpm.cjs`). `engines.node` passe à `>=24.12.0`, `make` vérifie la version de Node avant d'installer l'UI (`make check-node`) avec un message de remède, et le README décrit le dépannage (cache Corepack à effacer, réseau).

- Tests d'intégration instables à chaque changement de minute : l'heure murale du module Horloge est désormais injectée (`BuiltinServices::wall_clock`), figée dans `BuiltinServices::simulated()` ; `--demo` garde l'heure réelle.

### Modifié

- Les configurations existantes sont migrées à la lecture (schéma 1 vers 2) : les molettes Audio et Pomodoro gardent leur fonction, la molette Navigation devient la fonction du même nom, les autres (Horloge…) deviennent « Rien ».
- Pomodoro : le clic de la molette ouvre le menu (tourner ne l'ouvre plus) ; un clic sur une entrée la valide.
- Configuration d'exemple : fonctions de molette et réglages de la bande.
- Interface : les réglages d'un module s'enregistrent sans bouton « Appliquer » : un choix (case, liste, fichier validé ou retiré) tout de suite, un texte à la fin de la saisie (1 s sans frappe, sortie du champ ou Entrée), jamais pendant la frappe. Échap rétablit la valeur enregistrée d'un champ ; un champ invalide garde sa saisie et son erreur sans bloquer les autres, et un message avertit qu'elle est perdue si l'on passe à une autre touche ou molette. Une modification faite dans un autre onglet met à jour les champs non touchés (le bandeau « Recharger » disparaît).
- Interne : `docker/mod.rs` découpé en `settings`, `pages`, `actions`, `operation` et `render` ; la portée service d'une action compose est un paramètre (`ComposeScope`) et non plus un champ de `ComposeTarget` ; `FileEntry` lit seul l'ancien format (liste de chemins) ; `ConfigStore` prend les descripteurs des modules par un trait propre (`ModuleDescriptors`) au lieu de méthodes à corps vide de `ConfigDefaults`, `prepare_import` en est une méthode.
- Module Docker, page « Up » : les variantes se nomment Up, Rebuild, Pull et No start ; page « Down » : Down et Avec volumes (aussi pour le `down` d'un seul service).
- Interface web refondue : palette « thé et bergamote » (neutres teintés de vert, un seul accent chaud pour la sélection et l'action principale, thèmes clair et sombre), polices embarquées (Bricolage Grotesque, Instrument Sans, JetBrains Mono, sous-ensemble latin, aucun chargement externe), appareil dessiné comme un objet sombre à gauche et réglages de la touche ou de la molette choisie à droite. Une bande tactile est dessinée entre les touches et les molettes, avec l'icône du module de chaque molette (`DialStrip`) ; les molettes sont dessinées comme des molettes (`DialKnob`, repère bergamote) avec le nom de leur module, l'icône étant déjà sur la bande. Aucun changement de comportement ni de contrat.
- Nouvelle identité : logo (grille de touches formant un « H », molette au centre), favicon, bannière simple en en-tête de l'interface et du README : le nom seul, fond transparent, « B » en dégradé doux entre « Hub » et « Bergamof ». L'icône de la barre système n'est plus l'icône générique `input-keyboard` mais la marque, envoyée en pixmaps ARGB (22, 32, 48 et 64 px).

- Pomodoro : tourner la molette ouvre un menu à son propre visuel (titre avec le temps restant, entrée choisie sur une pastille) : « Mettre en pause » / « Reprendre » / « Démarrer », « Passer à la pause », « Remise à zéro » ; un clic exécute l'entrée choisie, sans ✓, et ferme le menu. Le clic de la molette, menu fermé, et la touche gardent leur appui unique pour la pause et la reprise. `ZoneContent::Menu` (`MenuZone`, trois entrées au plus) rejoint l'API des modules.
- Molettes Audio, Pomodoro et Accueil : nouveaux visuels de bande dans le ton des icônes. Audio : jauge en arc cyan avec le glyphe haut-parleur ; Pomodoro : anneau de progression avec le pictogramme de l'étape (tomate, tasse, pause) et une pastille par pomodoro du cycle ; Accueil : titre centré, parent en indigo, pastille de retour (le clic de la molette ne change pas) et ligne affichée en petits segments discrets en bas. API des modules : `GaugeContent` gagne `shape` (`Bar`, `Arc`, `Ring { steps }`), `accent` et `glyph` ; nouveau `ZoneContent::Page` ; `NavigationView::parent_title`.
- Touches : l'action se lance au **relâchement** (et non plus à l'appui) ; maintenue plus de 400 ms, la touche affiche son libellé en blanc sur noir sur la bande tactile et ne lance rien. Les molettes agissent toujours à l'appui.
- Pomodoro : réglages `notificationEnabled` (notification sur la bande, activée par défaut) et `notificationSeconds` (durée en secondes, 10 par défaut ; 0 = jusqu'à ce qu'on la ferme).
- Module Horloge : cadran « Éphéméride » sur la bande tactile (feuille de calendrier, heure en graisse fine, numéro de semaine ISO) ; avec les secondes, exposant teal et filet de progression de 60 s à repères de 15 s. Sans réglage supplémentaire.
- Touches : plus aucun cadre (l'attention garde son fond orange) ; libellé plus petit, sur une seule ligne, sous l'icône qui reste entièrement visible.
- Le cadre bleu des touches actives est supprimé (les icônes dédiées portent l'état) ; audio : l'entrée ou la sortie sélectionnée est marquée d'un badge tick vert.
- Icônes des modules : pictogrammes colorés sur fond noir, logos officiels pour Docker, IntelliJ et Claude, icônes distinctes pour conteneurs et projets compose Docker selon leur état.
- Projet renommé Bergadeck → HuBergamof (crates, binaire, service systemd, règle udev, documentation) ; renommage seul, sans changement fonctionnel.
- pnpm est épinglé avec son empreinte (`packageManager` de `ui/package.json` : `pnpm@12.16.1+sha512.…`) : Corepack refuse un pnpm dont le contenu diffère, au lieu de faire confiance au registre npm.

### Ajouté

- Notifications sur la bande tactile : tout module peut en envoyer (`Command::Notify`, message, couleur de fond, délai optionnel) ; elle prend toute la bande (icône du module, message qui défile s'il est trop long, filet de temps restant) et s'enlève au délai, au toucher de la bande (lue) ou au clic de la molette Navigation (non lue) ; les autres molettes sont inactives pendant l'affichage. Module « Notifications » (cloche, badge du nombre de non lues, liste des non lues qui se réaffichent sans délai). Pomodoro en envoie une à chaque fin d'étape.
- Nom de branche parlant obligatoire (`make check-branch`, inclus dans `make check`, étape de la CI) : `[claude/]<type>(/|-)<mots-en-kebab>` ; le nom aléatoire généré par l'outil est refusé. Scripts de contrôle testés (`make test-scripts`).
- API des modules : `ZoneContent::Clock` (`ClockFace`, `CalendarLeaf`) ; rendu : police Noto Sans Light (OFL) et choix de graisse du texte.
- Initialisation du projet : workspace Cargo, interface SvelteKit, outillage qualité, CI, empaquetage.
- Gestionnaire : couche appareils (Stream Deck matériel et virtuel `--demo`), rendu d'images, configuration par appareil.
- API des modules, moteur de pages, modules horloge et navigation.
- Serveur web local (UI embarquée, WebSocket, contrôle `Host`/`Origin`, CSP) et icône de barre système (zbus 6).
- Jauge sur la bande tactile (`ZoneContent::Gauge`).
- API des modules : événements des tâches de fond (`EventSender`, `on_event`), validation des réglages avant enregistrement, état visuel des touches (`KeyState`) et retour ✓/⚠ transitoire (`Command::Feedback`).
- Module Audio (PipeWire/PulseAudio via `pactl`, simulé en `--demo`) : volume, sourdine, sortie et entrée par défaut ; un seul abonnement partagé par tous les appareils.
- Module Pomodoro avec notifications de bureau ; le minuteur suit le temps réel, mise en veille comprise.
- API des modules : `ModuleDescriptor::parse_settings` (bornes vérifiées puis désérialisation) et `TestContext` (feature `testing`).
- Lint : lancement de shell interdit hors tests ; `module-api` vérifié seul, sans features.
- Tests e2e : contrôle d'accessibilité axe (WCAG 2.2 A/AA) des états riches de l'interface, en thème clair et sombre.
- Module Docker : projets compose et conteneurs (démarrer, arrêter, redémarrer, supprimer arrêté, Up / Up + build / Up + pull, Down / Down + volumes avec double appui), état en direct, fichiers compose déclarés (`composeFiles`) proposés même après un down ; simulé en `--demo`.
- Interface de configuration : choix de l'appareil, luminosité, touches de l'accueil (ajout depuis le catalogue, réorganisation au clavier et à la souris, suppression), affectation des molettes, réglages de chaque module ; synchronisée entre onglets, erreurs du gestionnaire affichées.
- API des modules : `ModuleFactory::icon`, l'icône affichée par l'interface ; le catalogue (`CatalogModule`) la fournit rendue en PNG.
- Image personnalisée des touches (PNG, JPEG ou SVG), envoyée depuis l'interface et affichée rendue par le gestionnaire.
- Export de la configuration d'un appareil en zip (`config.json` lisible + `images/`) et import, y compris vers un autre modèle (molettes adaptées), après confirmation.
- Sécurité : routes `/api` refusées quand un autre site les déclenche (`Sec-Fetch-Site`) ; détection SVG stricte ; images inutilisées supprimées au chargement de la configuration.
- Couche bureau des modules (socle des futurs modules IntelliJ et Claude Desktop) : fenêtres suivies et activées via KWin, applications détectées par leur exécutable, notifications de ces applications observées sur le bus de session (attribuées par l'exécutable de l'expéditeur, les autres jamais conservées), lancement hors du service systemd ; simulée en `--demo`.
- Module IntelliJ : touche d'accueil (lance IntelliJ s'il est fermé, en attention quand une fenêtre le demande), page des projets récents (ouverts d'abord) pour passer à la fenêtre d'un projet ou l'ouvrir ; touche en attention quand IntelliJ envoie une notification en arrière-plan (tests lancés, build terminé) jusqu'à l'appui ou son retour au premier plan ; installation Toolbox ou `/opt` détectée, lanceur vérifié avant usage ; simulé en `--demo`.
- Module Claude Desktop : une touche qui lance Claude s'il est fermé ou seulement dans la barre système, et passe à sa fenêtre sinon ; en attention quand Claude Desktop ou claude.ai dans Brave envoie une notification en arrière-plan, jusqu'à l'appui ou son retour au premier plan ; un lancement à la fois ; simulé en `--demo`.

- API des modules : `Command::alert()`, `Feedback::of(ok)` et `KeyContent::with_icon`, utilitaires communs aux modules.
- Qualité : couverture des tests mesurée (cargo-llvm-cov, Vitest v8) avec plancher en cliquet ; tests de composants Svelte dans un vrai navigateur ; tests du module Docker contre un vrai Docker (`make test-docker`, job CI dédié) ; audit des workflows par zizmor.
- Interface servie précompressée (brotli, gzip) aux navigateurs qui l'acceptent.
### Modifié

- Performances : processus regardés moins souvent quand rien ne change (de nouveau souvent dès qu'une fenêtre apparaît ou disparaît, ou après un lancement) ; cache de rendu réduit (192 images réutilisées) et data URL plus gardées en mémoire.
- Sécurité : configuration bornée (instances, touches, molettes, libellés de 64 caractères, réglages de 16 Kio, identifiants `[A-Za-z0-9_-]`) ; erreurs internes masquées aussi sur le WebSocket ; dossiers de configuration en 0700 (ceux d'un appareil existant y sont ramenés au chargement) ; archives à fin ambiguë (octets après le commentaire compris) ou ZIP64 refusées ; 512 images au plus par appareil ; l'interface ouverte depuis le tray démarre dans son propre scope systemd.
- Interface : supprimer une touche dont le module n'est placé nulle part ailleurs demande confirmation (ses réglages seraient perdus) ; le focus va sur la touche ajoutée, et revient sur le contrôle d'origine après une reconnexion au gestionnaire.
- Contrat : `moduleCatalog` envoie des `CatalogModule` (descripteur, icône, réglages par défaut choisis par le module) ; `selectDevice` porte `liveState` : l'état en direct n'est plus envoyé qu'aux clients qui le demandent.
- API des modules : un réglage chemin (`path`, `pathList`) doit être absolu ou commencer par `~/`.
- Tests e2e et Lighthouse lancés contre `hubergamof --demo` ; Lighthouse exige aussi Performance ≥ 0,9 ; un test e2e qui ne passe qu'à la relance fait échouer la CI.
- CI : toutes les actions épinglées par SHA ; gitleaks et actionlint vérifiés contre une somme SHA-256 épinglée ; binaire release sans table des symboles.
- Contrat : `KeyView.active` remplacé par `state` (`normal`/`active`/`attention`) ; `Command::Redraw` et `ImageFormat` supprimés.
- Isolation des modules : panique → placeholder et alerte, budget de 500 ms par gestionnaire ; appui résolu sur ce qui est affiché.
- Icône de barre système : plus d'avertissement `Broken pipe` à l'arrêt.
- Configuration d'exemple : l'horloge passe de l'accueil à la molette 4.
- Audio : un cran de molette coûte 2 appels `pactl` au lieu de 7 ; le volume envoyé est la valeur finale (jamais au-delà du plafond) ; la baisse va jusqu'au silence même si la balance est inégale ; une modification faite pendant le redémarrage du worker audio n'est plus perdue.
- Rendu : les images éphémères (décompte, jauges) ne chassent plus les autres du cache (mémoire stable pendant un pomodoro).
- Les tests et le mode `--demo` n'utilisent que des services simulés (aucun vrai `pactl` ni D-Bus).
- Retour ✓/⚠ affiché seulement sur une surface visible.
- Sécurité : hors `--demo`, le serveur refuse les appuis simulés (`InputRefused`) ; les actions ne viennent que de l'appareil.
- Service systemd lié à `graphical-session.target` ; image Docker sans `libhidapi`, lancée en `--demo --no-tray` (`--network host`).

### Supprimé

- Modules Horloge et Navigation : leurs rôles sont tenus par la bande (heure, date, page) et par la fonction de molette Navigation. API des modules : `Module::zone`, `ZoneContent` (jauges, horloge, page, menu de zone), `NavigationView`, `ModuleFactory::dismisses_notifications`, `ModuleCapabilities::dial` et `touch_zone`. Contrat : `LiveState::dials` (remplacé par l'image de la bande).

