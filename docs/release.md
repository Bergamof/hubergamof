# Release et paquets

## Lancer une release

GitHub → Actions → **Release** → *Run workflow*, branche **main** (toute autre branche est refusée).
Choix « Niveau de version » : `auto` (par défaut, d'après les commits), `patch`, `minor` ou `major`. Le niveau choisi est un **minimum** : il permet de publier quand les commits ne contiennent que du `chore`, `ci`, `docs`, `refactor`…, ou de monter d'un cran (une majeure sans commit `!`), mais ne descend jamais sous ce qu'imposent les commits (`patch` avec un `feat` donne une mineure).

Le workflow `.github/workflows/release.yml` enchaîne quatre jobs :

1. **gate** — fige le SHA de `main` ; attend la fin du dernier run de `CI` sur ce SHA et sur `main` (événements `push` et planification ; jamais un run de PR) et échoue s'il est en erreur, annulé ou introuvable ; calcule la version.
2. **build** (Ubuntu 24.04, sans cache) — fixe la version dans `Cargo.toml`/`Cargo.lock` et dans le manifeste de l'extension Teams `ui/teams-extension/manifest.json` (copie de travail, jamais committée ; l'extension embarquée porte ainsi celle du paquet), `make build`, `make package`, `lintian`, puis installe chaque paquet dans un conteneur propre (`make package-smoke`) ; produit les paquets.
3. **sign** (environnement `release`, runner neuf) — télécharge les paquets, calcule `SHA256SUMS` et le **signe** (`SHA256SUMS.sig`, voir « Signature »). Seul job qui voit la clé privée : il n'exécute ni `make`, ni pnpm, ni cargo, donc aucune dépendance du projet ne tourne à côté de la clé.
4. **publish** — crée d'abord le tag `vX.Y.Z` au SHA vérifié par l'API (échec s'il existe déjà), puis la release avec `--verify-tag` : livrables, `SHA256SUMS`, `SHA256SUMS.sig` et note.

## Numéro de version

`.github/scripts/next-version.sh`, d'après les Conventional Commits depuis le dernier tag `vX.Y.Z` (pré-versions ignorées) :

| Commits | Version |
|---|---|
| aucun tag | `1.0.0` |
| `type!:` ou `BREAKING CHANGE:` dans le corps | majeure |
| `feat` | mineure |
| `fix`, `perf` | patch |
| seulement `chore`, `ci`, `docs`, `refactor`… | refus « rien à publier » en `auto` |
| aucun commit depuis le tag | refus, quel que soit le niveau choisi |

Puis le niveau choisi (`--bump patch|minor|major`) s'applique s'il est plus haut que celui des commits ; sans tag, toujours `1.0.0`.

Un sujet qui ne suit pas la convention ne déclenche aucun bump. Les titres de PR (fusion en squash) deviennent les sujets : les soigner.

## Livrables

| Fichier | Cible |
|---|---|
| `hubergamof_X.Y.Z_amd64.deb` | Ubuntu 24.04+, Debian 12+ |
| `hubergamof-X.Y.Z-1-x86_64.pkg.tar.zst` | Manjaro, Arch Linux (`pacman -U`) |
| `SHA256SUMS` | sommes des deux paquets |
| `SHA256SUMS.sig` | signature Ed25519 de `SHA256SUMS` (64 octets), vérifiée par l'assistant de mise à jour |

Les deux contiennent le même binaire (UI embarquée), construit sur Ubuntu 24.04 : sa glibc est assez ancienne pour Manjaro stable, que ses paquets ne mettent pas à jour aussi vite qu'Arch.

**Dépendances obligatoires** : bibliothèques du binaire (`libudev`), udev, systemd, `xdg-utils`, et pour le module Audio `pactl` (Debian : `pulseaudio-utils (>= 1:16)` ; Arch : `libpulse`) avec un serveur de son (`pipewire-pulse | pulseaudio` ; Arch : `pulse-native-provider`).
**Optionnelles** (`Suggests`/`optdepends`) : Docker, `xdg-desktop-portal` (sélecteur de fichier), extension AppIndicator de GNOME.

**Fichiers installés** : `/usr/bin/hubergamof`, `/usr/lib/udev/rules.d/70-hubergamof.rules`, `/usr/lib/systemd/user/hubergamof.service` (configuration dans `~/.config/hubergamof`).

**Scripts de maintenance** (`packaging/package/hooks.sh`, communs aux deux formats) :
- installation : rechargement d'udev et évènements `change` limités aux appareils Elgato (un appareil branché est accessible sans débranchement), `systemctl --global enable` (actif pour tous les utilisateurs au prochain login), puis `daemon-reload` et `restart` dans chaque session graphique déjà ouverte (`systemctl --user --machine=<utilisateur>@.host`) ;
- mise à jour : idem, mais `try-restart` : un service arrêté volontairement le reste ;
- retrait : arrêt dans les sessions, désactivation globale, rechargement d'udev ;
- aucune étape ne fait échouer l'opération ; la configuration de l'utilisateur n'est jamais supprimée.

Le service est une unité *utilisateur* : sans session graphique ouverte, il démarre au prochain login.

## Signature et mises à jour automatiques

Un gestionnaire installé par paquet cherche une nouvelle release (API GitHub, sans jeton), télécharge `SHA256SUMS`, `SHA256SUMS.sig` et son paquet, puis lance par `pkexec` l'assistant `/usr/lib/hubergamof/hubergamof-update-helper` (root, action polkit `io.github.bergamof.hubergamof.update`). L'assistant ne se fie à rien de ce qu'il reçoit : il vérifie la signature de `SHA256SUMS` avec la **clé publique embarquée** (`crates/manager/assets/release-signing.pub`), l'empreinte du paquet contre `SHA256SUMS`, refuse toute version qui n'est pas plus récente que celle installée (rejeu d'une ancienne release signée), puis passe le paquet vérifié à `apt-get install` ou `pacman -U`. Le hook du paquet redémarre ensuite le service.

- **Clé privée** : secret `RELEASE_SIGNING_KEY` (PEM), à ranger dans l'**environnement GitHub `release`** (Settings → Environments → `release` : branches autorisées `main`, relecteur obligatoire ; y créer le secret puis supprimer le secret de dépôt, lisible sinon par les workflows de toute branche). `.github/scripts/sign-checksums.sh` signe, puis vérifie la signature avec la clé publique du dépôt : un secret qui n'est pas la clé publiée fait échouer la release (jamais de release non signée ni signée par une autre clé).
- **Créer la paire** (sur un poste de confiance, sans phrase de passe) :
  ```sh
  openssl genpkey -algorithm ed25519 -out release-signing.key
  openssl pkey -in release-signing.key -pubout -out release-signing.pub
  ```
  La clé publique va dans `crates/manager/assets/release-signing.pub` (commitée), la privée dans le secret, puis le fichier est supprimé du poste (copie dans un gestionnaire de mots de passe).
- **Limite actuelle** : une seule clé embarquée, sans liste de révocation ni version minimale signée ; un attaquant maître du réseau ne peut pas faire installer une version plus ancienne que celle installée, mais peut choisir n'importe quelle release signée plus récente (y compris intermédiaire). Piste : liste de clés (courante + suivante) et version minimale dans le contenu signé.
- **Rotation / perte de la clé** : les installations existantes n'acceptent que la clé embarquée dans leur binaire. Pour changer de clé, publier d'abord une release signée par l'ancienne clé et embarquant la nouvelle ; une clé perdue impose une réinstallation manuelle.
- Installation depuis les sources (`make install`) : pas d'assistant, donc vérification seule ; `--no-update-check` supprime toute connexion sortante.

## Construire les paquets en local

```sh
make build
make package          # .deb (dpkg-deb) + pacman (conteneur Arch) dans dist/
make package-lint     # lintian
make package-smoke    # installation puis retrait dans des conteneurs Ubuntu 24.04 et Arch
```

`make package-arch` et `package-smoke` demandent Docker. La version vient de `Cargo.toml` (`0.1.0` en local ; la release la fixe d'après le tag).

## Pièges

- Les images de `packaging/package/{arch,ubuntu}/Dockerfile` sont épinglées par digest (Dependabot).
- Le build de release n'utilise aucun cache (Rust, pnpm) : une release ne repart jamais d'un cache.
- Le dernier run de CI (push ou planification) compte, même hebdomadaire : une dérive de dépendances bloque la release jusqu'à correction.
- `make install` (depuis les sources, `~/.local/bin`) et le paquet peuvent cohabiter : l'unité de `~/.config/systemd/user` l'emporte sur celle du paquet.

## Décisions et pistes de sécurité (revue cerbere-le-paranoiaque)

- **Service activé pour tous les comptes** : voulu (« le paquet crée et démarre le service »), hypothèse du projet : poste mono-utilisateur, voir `docs/security.md`.
- **Pistes écartées** (usage personnel, postes mono-utilisateur) : attestation de provenance des livrables (`actions/attest-build-provenance`, SHA à épingler ; indisponible sur dépôt privé sans Enterprise) ; `cargo deny`/`pnpm audit` rejoués dans `gate` ; ruleset GitHub protégeant les tags `v*` (réglage du dépôt, à faire à la main si besoin). À rouvrir si le dépôt devient public.
- **Fait** : pnpm épinglé avec son empreinte sha512 (`packageManager` de `ui/package.json`), vérifiée par Corepack.
- Les sujets de commit passent tels quels dans les notes, sauf `@nom` mis en code.
