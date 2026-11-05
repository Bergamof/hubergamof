# Sécurité : risques acceptés et décisions

Complète les décisions de `CLAUDE.md` et les contraintes de `integrations-desktop.md`.

## Risques acceptés
- **Autres comptes Unix du poste** (revue sécurité étape 9) : le serveur n'a pas de jeton de session (décision du 2026-10-28). Sur un poste partagé, un autre compte local qui forge `Host`/`Origin` (curl) peut :
  - lire la configuration et le `liveState` (chemins de projets IntelliJ, noms de conteneurs) ;
  - remplacer la configuration : une touche à l'image « Muet » qui lance en fait `docker compose down --volumes`, piège qui attend un appui physique (même leurre possible via un zip d'export partagé) ;
  - téléverser des images (bornées en taille et en nombre : `MAX_STORED_IMAGES` par appareil).

  Rien ne s'exécute sans appui physique, et un processus du même compte a de toute façon tous les droits. Accepté pour un poste mono-utilisateur (Adrien, 2026-11-05). À revoir si le poste devient multi-comptes ou si l'UI gagne des actions directes : jeton dans un fichier 0600 de `$XDG_RUNTIME_DIR`, transmis à l'UI par l'URL ouverte depuis le tray, ou socket Unix + proxy.

## Décisions
- **Durcissement de l'unité systemd** : ni `UMask=0077` ni `NoNewPrivileges=yes`. Les applications lancées par le gestionnaire (IntelliJ, Claude Desktop, navigateur du tray) passent par `systemd-run --scope`, qui ne fait que les déplacer de cgroup : elles restent filles du service et hériteraient de l'umask (fichiers des projets créés en 0600) et de `NoNewPrivileges` (bac à sable setuid d'Electron/Chromium cassé). À la place, le `ConfigStore` crée ses dossiers en 0700 et y ramène, au chargement d'un appareil, son dossier et `images/` créés par une version antérieure (fichiers déjà en 0600 ; `config/` et `config/devices/` existants restent à `chmod go-rwx` une fois). Options de bac à sable (`ProtectSystem`…) exclues : le gestionnaire lit `/proc`, les sockets Docker et D-Bus, `~/.config/JetBrains`.
- **Configuration bornée** (`DeviceConfig::validate`) : instances, touches, molettes, libellés, taille des réglages, identifiants (`[A-Za-z0-9_-]`, 64 au plus, aussi pour les types de module) : une configuration importée ou reçue ne peut ni figer le gestionnaire à chaque démarrage ni injecter de fausses lignes dans les journaux. Titres et ids de page journalisés échappés (`?`).
- **Erreurs internes** : message générique au client, en HTTP comme en WebSocket (`server/errors.rs`), détail dans les journaux.
- **Archives** : un seul enregistrement de fin d'archive cohérent (commentaire jusqu'à la fin du fichier), sans ZIP64, sinon refus avant toute analyse du répertoire central.
