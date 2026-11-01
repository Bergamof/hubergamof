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
- Valider avec `actionlint` et `shellcheck` ; `make` doit fonctionner depuis une machine propre avec les prérequis du README.
