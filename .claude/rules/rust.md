---
paths:
  - "crates/**"
  - "Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
  - "rustfmt.toml"
  - "clippy.toml"
  - "deny.toml"
  - ".cargo/**"
  - "config/example/**"
---

# Rust (gestionnaire, modules, contrat)

## Règles
- Dépendances partagées dans `[workspace.dependencies]` ; toute nouvelle dépendance doit passer `cargo deny check` (licence compatible GPL, MPL pour `module-api`).
- Avant de rendre la main : `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo deny check` (ou `make lint-rust test-rust deny`).
- Pas d'`unwrap`/`expect` hors tests, sauf invariant documenté ; `thiserror` dans les bibliothèques, `anyhow` au niveau du binaire.
- Le matériel reste derrière `DeckBackend` ; tout comportement se teste sur le backend virtuel.
- Commandes externes : `std::process::Command` / `tokio::process::Command` avec arguments séparés, jamais de chaîne passée à un shell.
- Serveur : `127.0.0.1` uniquement + contrôle `Host`/`Origin` (pas de jeton de session, cf. décisions de `CLAUDE.md`).

## Pièges
- Tests d'intégration marqués `#![cfg(test)]` pour que `allow-unwrap-in-tests` de clippy s'applique.
- Tests manuels du binaire : `--config-dir` vers un dossier temporaire, jamais `config/` du dépôt.
- Toute modification d'un type du contrat (y compris sa rustdoc) impose `make gen-types` dans le même commit.
- Tests asynchrones des modules en temps simulé (`#[tokio::test(start_paused = true)]`), jamais en temps réel (instable sous charge) : `spawn_blocking` est compatible, tokio suspend l'avance automatique du temps tant qu'une tâche bloquante tourne. Attendre une condition (événement, état) plutôt qu'une durée.
