---
paths:
  - ".github/**"
  - "Dockerfile"
  - ".dockerignore"
  - "Makefile"
  - "packaging/**"
  - "lighthouserc.json"
---

# CI/CD et packaging

## Règles
- Versions lues depuis `rust-toolchain.toml`, `.nvmrc`, `packageManager` ; les mêmes dans le `Dockerfile` (`make check-versions`).
- Workflows : `concurrency`, permissions minimales par job, `outputs.summary` par job et job `report` conformes à `.github/workflows/ci.yml` ; dernière version majeure de chaque action, vérifiée, épinglée par SHA.
- Jamais de `pull_request_target` avec checkout du code de la PR.
- Dockerfile multi-stage, image finale minimale, utilisateur non-root ; sert au build reproductible et au mode `--demo`, pas à l'exécution réelle.
- systemd : unité **utilisateur** ; udev : `TAG+="uaccess"` (jamais `MODE=0666`).
- Valider avec `actionlint`, `shellcheck` et `zizmor` (`make lint-workflows`, config `.github/zizmor.yml`) ; `make` doit fonctionner depuis une machine propre avec les prérequis du README.

## Pièges
- Le Makefile appelle `check-node` avant `pnpm install` : pnpm 12 ne démarre pas sous Corepack < 0.34.5 (Node < 24.12), donc `engines` ne peut pas s'en charger. Relever le minimum = changer `engines.node` seulement (le script le lit).
- Les scripts de `.github/scripts/` sont couverts par shellcheck en CI et par `test-control-scripts.sh` : tout nouveau script de contrôle y ajoute ses cas.
- Outil téléchargé hors action (`install-release-binary.sh`) : somme SHA-256 épinglée dans le workflow, jamais vérifiée contre le fichier de sommes de la même release.
- zizmor `self-repository` (`uses: $/…`) désactivé : actionlint 1.7.12 refuse cette syntaxe ; à réactiver avec un actionlint qui la connaît.
- Job `test` : `cargo llvm-cov` remplace `cargo test` (mêmes tests) ; le TOTAL donne régions, fonctions puis lignes (dernier pourcentage).
- pnpm épinglé avec empreinte (`pnpm@X+sha512.…`, vérifiée par Corepack : une empreinte fausse donne « Mismatch hashes ») : changer la version sans l'empreinte fait échouer `make ui-deps`, le job `docker` et `pnpm/action-setup` ; `corepack use pnpm@<version>` depuis `ui/` écrit les deux.
