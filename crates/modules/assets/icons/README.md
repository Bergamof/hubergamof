# Icônes des modules

SVG 128×128, pictogramme coloré sur fond transparent (l'hôte les pose sur le fond du thème de l'appareil, clair ou sombre), embarqués par `include_str!` (`src/icons.rs`).

- Logos Docker, IntelliJ IDEA et Claude : glyphes officiels de [Simple Icons](https://simpleicons.org) (CC0), recolorés en dégradé. Les marques appartiennent à leurs titulaires ; usage personnel, non distribué (décision d'Adrien, 2026-11-05) — à revoir si le dépôt devient public.
- Les autres icônes sont dessinées pour le projet.
- Chaque fichier est autonome (ses dégradés sont définis dans le fichier) ; ne jamais peindre de fond (test `no_icon_paints_a_background`) ; les découpes derrière un badge ou un trait barré sont des masques SVG (`<mask id="k">`), pas des aplats noirs. Vérifier toute nouvelle icône sur fond clair et sombre.
- Pictogrammes `*-glyph*.svg` : sans fond, ajustés à leur dessin (viewBox serré), dessinés dans la jauge d'une zone de la bande par l'hôte (anneau du pomodoro, titre de la jauge audio).
