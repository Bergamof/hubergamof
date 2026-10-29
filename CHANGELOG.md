# Changelog

Toutes les évolutions notables de ce projet sont consignées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet respecte le [versionnement sémantique](https://semver.org/lang/fr/).

## [Non publié]

### Ajouté

- Initialisation du projet : workspace Cargo, interface SvelteKit, outillage qualité, CI, empaquetage.
- Gestionnaire : couche appareils (Stream Deck matériel et virtuel `--demo`), rendu d'images, configuration par appareil.
- API des modules, moteur de pages, modules horloge et navigation.
- Serveur web local (UI embarquée, WebSocket, contrôle `Host`/`Origin`, CSP) et icône de barre système (zbus 6).
- Jauge sur la bande tactile (`ZoneContent::Gauge`).
- API des modules : événements des tâches de fond (`EventSender`, `on_event`), validation des réglages avant enregistrement, état visuel des touches (`KeyState`) et retour ✓/⚠ transitoire (`Command::Feedback`).

### Modifié

- Contrat : `KeyView.active` remplacé par `state` (`normal`/`active`/`attention`) ; `Command::Redraw` et `ImageFormat` supprimés.
- Isolation des modules : panique → placeholder et alerte, budget de 500 ms par gestionnaire ; appui résolu sur ce qui est affiché.
- Icône de barre système : plus d'avertissement `Broken pipe` à l'arrêt.
- Configuration d'exemple : l'horloge passe de l'accueil à la molette 4.
- Service systemd lié à `graphical-session.target` ; image Docker sans `libhidapi`, lancée en `--demo --no-tray` (`--network host`).
