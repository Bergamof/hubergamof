# Sonde des souris Logitech (`mouse-probe`)

Étape 0 du support des souris MX : la sonde dit ce que chaque souris annonce d'elle-même, afin de valider les boutons, les réglages et l'haptique **sur le matériel** avant de les coder.

## Ce qu'elle fait
- Ouvre les interfaces HID++ Logitech (récepteurs Logi Bolt et Unifying, câble) et interroge chaque appareil : version du protocole, nom, liste des fonctions (`FeatureSet`), boutons réaffectables (`ReprogControlsV4`).
- **Lecture seule** : elle n'envoie que des fonctions de lecture (ping, `getFeature`, nombre et description des fonctions et des boutons, nom). Elle ne modifie aucun réglage, ne détourne aucun bouton et ne déclenche aucun effet.
- N'ouvre que les collections HID++ (voir plus bas) et n'envoie rien aux interfaces de saisie. Sur un récepteur **Unifying** le nœud HID++ fait aussi passer la saisie des appareils appairés (canal DJ) : ne l'ouvrez pas à d'autres programmes que la sonde. Sur le **Logi Bolt**, l'interface HID++ ne porte pas la saisie, mais l'accès en écriture a ses risques (`docs/security.md`).

## Donner l'accès à la sonde
La règle udev du projet (`make install` ou le paquet) ouvre l'interface HID++ d'un récepteur **Logi Bolt** à l'utilisateur de la session (voir `docs/security.md`) ; débrancher puis rebrancher le récepteur après l'installation. Pour un autre récepteur, deux façons, sans lancer la sonde en root (en root elle écrirait aussi sur des interfaces qu'on n'a pas à toucher) :

- **Solaar installé** : ses règles udev donnent déjà l'accès, rien à faire. S'il tourne, arrêtez-le pour la sonde (`systemctl --user stop solaar`).
- **ACL temporaire** : lancer la sonde une première fois, elle liste les interfaces (`not probed`, `cannot open: …`) ; repérer celle du récepteur Logitech dont la page d'usage est `ff00`, usage `0001` ou `0002`, puis donner l'accès à ce seul nœud jusqu'au prochain débranchement :

```bash
sudo setfacl -m u:$USER:rw /dev/hidrawN
```

## La lancer
Souris allumée et appairée au récepteur. Une souris endormie ne répond pas : la sonde réessaie pendant quelques secondes chaque emplacement silencieux, **bougez les souris pendant ce temps**. Pour chaque emplacement sans appareil, la sortie donne la raison (« refused … resource error » : rien d'appairé à cet emplacement ; « no answer after N pings » : appairé mais endormi, éteint ou sur un autre canal) :

```bash
hubergamof mouse-probe
```

En cas d'anomalie, ajouter les trames échangées :

```bash
hubergamof mouse-probe --trace
```

La sonde n'ouvre que les collections HID++ de Logitech (pages `0xFF00`, usages 1 et 2, et `0xFF43`, usage `0x0202` ; jamais une interface de saisie ni un chargeur de micrologiciel). Les autres interfaces Logitech sont listées avec la mention « not probed ».

## Ce qu'il faut renvoyer
Toute la sortie du terminal. Elle contient le modèle, les fonctions (avec celles que le projet ne connaît pas, marquées `?`, parmi lesquelles se trouve l'haptique) et les boutons (identifiant, drapeaux : réaffectable, détournable…).
