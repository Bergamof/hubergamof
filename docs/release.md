# Release et paquets

## Lancer une release

GitHub → Actions → **Release** → *Run workflow*, branche **main** (toute autre branche est refusée).
Case « Publier un patch même sans feat, fix ni perf » : pour publier quand les commits depuis le dernier tag ne contiennent que du `chore`, `ci`, `docs`, `refactor`…

Le workflow `.github/workflows/release.yml` enchaîne trois jobs :

1. **gate** — fige le SHA de `main` ; attend la fin du dernier run de `CI` sur ce SHA (quel que soit l'événement : push, PR, planification) et échoue s'il est en erreur, annulé ou introuvable ; calcule la version.
2. **build** (Ubuntu 24.04, sans cache) — fixe la version dans `Cargo.toml`/`Cargo.lock` (copie de travail, jamais committée), `make build`, `make package`, `lintian`, puis installe chaque paquet dans un conteneur propre (`make package-smoke`) ; produit `SHA256SUMS`.
3. **publish** — `gh release create vX.Y.Z --target <SHA>` : crée en une opération la release, le tag, les livrables et la note.

## Numéro de version

`.github/scripts/next-version.sh`, d'après les Conventional Commits depuis le dernier tag `vX.Y.Z` (pré-versions ignorées) :

| Commits | Version |
|---|---|
| aucun tag | `1.0.0` |
| `type!:` ou `BREAKING CHANGE:` dans le corps | majeure |
| `feat` | mineure |
| `fix`, `perf` | patch |
| seulement `chore`, `ci`, `docs`, `refactor`… | refus « rien à publier » (sauf option de forçage : patch) |
| aucun commit depuis le tag | refus, même avec le forçage |

Un sujet qui ne suit pas la convention ne déclenche aucun bump. Les titres de PR (fusion en squash) deviennent les sujets : les soigner.

## Livrables

| Fichier | Cible |
|---|---|
| `hubergamof_X.Y.Z_amd64.deb` | Ubuntu 24.04+, Debian 12+ |
| `hubergamof-X.Y.Z-1-x86_64.pkg.tar.zst` | Manjaro, Arch Linux (`pacman -U`) |
| `SHA256SUMS` | sommes des deux paquets |

Les deux contiennent le même binaire (UI embarquée), construit sur Ubuntu 24.04 : sa glibc est assez ancienne pour Manjaro stable, que ses paquets ne mettent pas à jour aussi vite qu'Arch.

**Dépendances obligatoires** : bibliothèques du binaire (`libudev`), udev, systemd, `xdg-utils`, et pour le module Audio `pactl` (Debian : `pulseaudio-utils (>= 1:16)` ; Arch : `libpulse`) avec un serveur de son (`pipewire-pulse | pulseaudio` ; Arch : `pulse-native-provider`).
**Optionnelles** (`Suggests`/`optdepends`) : Docker, `xdg-desktop-portal` (sélecteur de fichier), extension AppIndicator de GNOME.

**Fichiers installés** : `/usr/bin/hubergamof`, `/usr/lib/udev/rules.d/70-hubergamof.rules`, `/usr/lib/systemd/user/hubergamof.service` (configuration dans `~/.config/hubergamof`).

**Scripts de maintenance** (`packaging/package/hooks.sh`, communs aux deux formats) :
- installation/mise à jour : rechargement d'udev et rejeu des évènements (un appareil branché est accessible sans débranchement), `systemctl --global enable` (actif pour tous les utilisateurs au prochain login), puis `daemon-reload` et `restart` dans chaque session graphique déjà ouverte (`systemctl --user --machine=<utilisateur>@.host`) ;
- retrait : arrêt dans les sessions, désactivation globale, rechargement d'udev ;
- aucune étape ne fait échouer l'opération ; la configuration de l'utilisateur n'est jamais supprimée.

Le service est une unité *utilisateur* : sans session graphique ouverte, il démarre au prochain login.

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
- Le dernier run de CI compte même s'il vient de la planification hebdomadaire : une dérive de dépendances bloque la release jusqu'à correction.
- `make install` (depuis les sources, `~/.local/bin`) et le paquet peuvent cohabiter : l'unité de `~/.config/systemd/user` l'emporte sur celle du paquet.
