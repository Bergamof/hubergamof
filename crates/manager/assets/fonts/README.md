# Polices embarquées

Noto Sans, version 2.015, sous [SIL Open Font License 1.1](OFL.txt) (texte de la licence, à ne pas modifier). Fichiers utilisés tels que publiés, sans sous-ensemble ni renommage.

| Fichier | Graisse | Source | SHA-256 |
|---|---|---|---|
| `NotoSans-SemiBold.ttf` | 600, partout | Projet Noto (`notofonts/latin-greek-cyrillic`) | `a7af8d4a11e8e49fea9a19786a6469210b512cd4061413e8036bdd2425722795` |
| `NotoSans-Light.ttf` | 300, chiffres de l'horloge | paquet npm `@expo-google-fonts/noto-sans` 0.4.2, `300Light/NotoSans_300Light.ttf` (« MIT AND OFL-1.1 », même version Noto) | `ad161dc21b079cabda74ba05032d63f729fb54ed899e995401bde1e49e19872e` |

Les deux fichiers ont les mêmes métriques (`hhea` 1069 / −293 pour 1000 unités par em), sur lesquelles repose `render/text.rs`. Une montée de version se fait pour les deux ensemble.
