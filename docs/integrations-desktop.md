# Intégrations bureau (résultats des spikes du 2026-10-28)

## Claude Desktop — « Claude demande quelque chose »
- Mécanisme : connexion D-Bus de session en monitor (`org.freedesktop.DBus.Monitoring.BecomeMonitor`) avec règle `interface='org.freedesktop.Notifications',member='Notify'` (+ signal `com.canonical.Unity.LauncherEntry`, jamais émis par Electron hors Unity mais écouté au cas où).
- Une connexion monitor ne peut plus rien envoyer : utiliser une 2ᵉ connexion pour `GetConnectionUnixProcessID`.
- Filtre : hint `desktop-entry == "com.anthropic.Claude"` (ou exécutable émetteur `/usr/lib/claude-desktop/claude-desktop`), jamais sur le texte (traduit, change selon les versions). Ignorer les messages `NameLost`/`NameAcquired`.
- Vie privée : le monitor voit toutes les notifications de la session ; ne rien journaliser hors du filtre Claude.
- Limite : pas de notification si la session concernée est déjà au premier plan. Signal complémentaire KDE : `demandsAttention` de la fenêtre `com.anthropic.Claude` (script KWin persistant, actif car `dockBounceEnabled=true`).
- Lancé ? processus dont `/proc/<pid>/exe` = `/usr/lib/claude-desktop/claude-desktop` sans `--type=`, ou fenêtre KWin `resourceClass == "com.anthropic.Claude"`. Lancement : `gtk-launch com.anthropic.Claude`. Relancer une appli ouverte sous Wayland ne fait que demander l'attention → activation par script KWin.
- Reste à confirmer manuellement : une vraie notification de demande de permission (champs `desktop-entry`, `summary`).

## IntelliJ
- Dossier de config : `dataDirectoryName` de `~/.local/share/JetBrains/Toolbox/apps/intellij-idea-ultimate/product-info.json` → `~/.config/JetBrains/<dataDirectoryName>/options/recentProjects.xml` ; repli : `IntelliJIdea*` le plus récemment modifié.
- `opened="true"` n'est PAS un état en direct (projets à rouvrir) : croiser avec les fenêtres KWin `resourceClass == "jetbrains-idea"` (`startupWmClass` de `product-info.json`) et le titre `projet – fichier` (séparateur ` – `). Le XML sert à retrouver le chemin.
- Détection du processus : `/proc/<pid>/exe` = `<install>/bin/idea` ou `com.intellij.idea.Main` dans `cmdline`. Piège : un `java` Toolbox (wiremock) utilise le JBR d'IntelliJ → faux positif si on cherche « intellij-idea » dans `cmdline`.
- Reste à confirmer avec IntelliJ ouvert : `resourceClass`, format du titre, mise à jour de `opened`, effet de `scripts/idea <chemin>` (probablement simple demande d'attention).

## KWin (KDE Plasma 6, Wayland)
- `org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript(path, plugin)` → id (-1 si déjà chargé) ; `/Scripting/Script{id}` `org.kde.kwin.Script.run` (**minuscule** : `#[zbus(name = "run")]`) ; `unloadScript(plugin)`.
- Retour vers Rust : le script appelle `callDBus(<nom unique :1.x>, path, iface, "Report", JSON.stringify(...))` sur un objet exposé par zbus.
- JS : `workspace.windowList()`, `workspace.activeWindow = w` (active réellement), `workspace.windowAdded/windowActivated`, `w.demandsAttentionChanged`, `w.captionChanged` ; propriétés `internalId` (`String()`), `resourceClass`, `desktopFileName`, `caption`, `pid`, `normalWindow`, `minimized`, `demandsAttention`.
- Robustesse : nom de plugin fixe par usage, `unloadScript` défensif au démarrage et déchargement garanti ; fichiers `.js` dans `$XDG_RUNTIME_DIR/bergadeck/` ; chaînes injectées échappées via `serde_json::to_string`.
- GNOME : pas d'API sans extension (« Window Calls » : `org.gnome.Shell.Extensions.Windows`), sinon simple clignotement (accepté).

## zbus 6
- `Value` → `Variant`/`OwnedVariant` ; plus d'API bloquante. Features utilisées : `default-features = false`, `tokio`, `proxy`, `service`. Nouvelles dépendances (`zruntime`, `zcheapstr`, `zbus_utils`) à passer dans `cargo deny`.
