---
paths:
  - "ui/**"
  - ".nvmrc"
---

# UI (SvelteKit / Svelte 5 / Tailwind)

## Règles
- Types du contrat exclusivement depuis `#lib/generated/index.js` ; ne jamais les redéfinir à la main ni éditer `ui/src/lib/generated/`.
- Svelte 5 runes uniquement (`$state`, `$derived`, `$props`, `$effect` avec parcimonie) ; aucune syntaxe Svelte 4.
- WCAG 2.2 AA : HTML sémantique, contraste, navigation clavier complète (y compris la réorganisation de l'accueil), focus visible, libellés. Objectif Lighthouse ≥ 90 Accessibilité et Bonnes pratiques.
- Interface en français ; états vide, chargement et erreur toujours traités (ex. « Aucun appareil connecté », perte de WebSocket avec reconnexion).
- Avant de rendre la main : `pnpm lint`, `pnpm check`, `pnpm test`, `pnpm build` (et `pnpm test:e2e` si l'UI a changé) — via `corepack pnpm -C ui …` ou `make lint-ui test-ui ui-build`.

## Pièges
- `adapter-static` : `fallback: '200.html'` (avec `index.html`, la page racine prérendue serait écrasée).
- pnpm 12 ignore `engine-strict` dans `.npmrc` : le réglage est dans `pnpm-workspace.yaml` (`engineStrict: true`).
- `$state.snapshot` sur un type contenant `JsonValue` (récursif) : « Type instantiation is excessively deep » → `cloneJson` (`#lib/deck/json.js`).
- Identifiants DOM : `$props.id()`, jamais d'id en dur dans un composant.
- Bouton qui devient inactif sous le focus (déplacer en bout de liste) : `aria-disabled` + action ignorée, pas `disabled` (le focus tomberait sur `body`).
- Fixture Playwright : le premier paramètre doit être déstructuré ; `({}, use)` est refusé par ESLint (`no-empty-pattern`) → dépendre d'une fixture utilisée (`browserName`…).
- Erreurs du gestionnaire : texte français + détail anglais dans `<span lang="en">` (`Problem`).
