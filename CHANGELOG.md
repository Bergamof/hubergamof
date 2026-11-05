# Changelog

Toutes les évolutions notables de ce projet sont consignées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet respecte le [versionnement sémantique](https://semver.org/lang/fr/).

## [Non publié]

### Modifié

- Le cadre bleu des touches actives est supprimé (les icônes dédiées portent l'état) ; audio : l'entrée ou la sortie sélectionnée est marquée d'un badge tick vert.
- Icônes des modules : pictogrammes colorés sur fond noir, logos officiels pour Docker, IntelliJ et Claude, icônes distinctes pour conteneurs et projets compose Docker selon leur état.
- Projet renommé Bergadeck → HuBergamof (crates, binaire, service systemd, règle udev, documentation) ; renommage seul, sans changement fonctionnel.

### Ajouté

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
