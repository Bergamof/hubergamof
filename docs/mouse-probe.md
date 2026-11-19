# Sonde des souris Logitech (`mouse-probe`)

Étape 0 du support des souris MX : la sonde dit ce que chaque souris annonce d'elle-même, afin de valider les boutons, les réglages et l'haptique **sur le matériel** avant de les coder.

## Ce qu'elle fait
- Ouvre les interfaces HID++ Logitech (récepteurs Logi Bolt et Unifying, câble) et interroge chaque appareil : version du protocole, nom, liste des fonctions (`FeatureSet`), boutons réaffectables (`ReprogControlsV4`).
- **Lecture seule** : elle n'envoie que des fonctions de lecture (ping, `getFeature`, nombre et description des fonctions et des boutons, nom). Elle ne modifie aucun réglage, ne détourne aucun bouton et ne déclenche aucun effet.
- N'ouvre aucune interface de saisie (clavier ou souris) : seules les interfaces d'usage HID++ (pages `0xFF00` et `0xFF43`) sont lues.

## La lancer
1. Installer la règle udev (`make install` ou le paquet), débrancher puis rebrancher le récepteur Logi Bolt (ou redémarrer la session). Sans règle, la sonde dit « cannot open: Permission denied » pour chaque interface.
2. Souris allumée et appairée au récepteur :

```bash
hubergamof mouse-probe
```

3. En cas d'anomalie, ajouter les trames échangées :

```bash
hubergamof mouse-probe --trace
```

Solaar et `logid` peuvent rester installés pour cette lecture, mais s'ils tournent ils lisent les mêmes trames : arrêtez-les si une réponse manque (`systemctl --user stop solaar`).

## Ce qu'il faut renvoyer
Toute la sortie du terminal. Elle contient le modèle, les fonctions (avec celles que le projet ne connaît pas, marquées `?`, parmi lesquelles se trouve l'haptique) et les boutons (identifiant, drapeaux : réaffectable, détournable…).
