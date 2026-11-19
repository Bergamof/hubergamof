# Souris Logitech : relevés HID++

Relevés de `hubergamof mouse-probe` du 2026-11-19, Logi Bolt (`046d:c548`, interface HID++ n° 2, collection `ff00/0001`). Les deux souris répondent en HID++ 4.5 aux emplacements 1 et 2 du récepteur ; les emplacements 3 à 6 sont vides (erreur HID++ 1.0 `0x09`).

## Récepteur
- Interfaces du Bolt : 0 clavier, 1 souris et commandes multimédia, **2 HID++ seul**, 3 numériseur. Pas de collection DJ.
- L'emplacement `0xFF` (le récepteur lui-même) ne répond qu'en HID++ 1.0 (erreur `0x01`).
- Un appareil endormi ne répond pas à la première demande : la sonde réessaie chaque emplacement muet pendant quelques secondes.

## Boutons réaffectables (`ReprogControlsV4`)
| Contrôle (cid) | MX Anywhere 3S (emplacement 1) | MX Master 4 (emplacement 2) |
|---|---|---|
| `0x0050` clic gauche | oui, non réaffectable | oui, non réaffectable |
| `0x0051` clic droit | oui, non réaffectable | oui, non réaffectable |
| `0x0052` clic molette | oui | oui |
| `0x0053` précédent | oui | oui |
| `0x0056` suivant | oui | oui |
| `0x00C3` bouton de geste | non | oui |
| `0x00C4` mode de molette (SmartShift) | oui (bouton du dessus) | oui |
| `0x01A0` inconnu | non | oui (réaffectable, détournable) : probablement le panneau haptique, à confirmer |
| `0x00D7` virtuel | oui (détournable) | oui (détournable) |

Drapeaux : `0x31` = souris + réaffectable + détournable ; `0x01` = souris seulement (clics gauche et droit, jamais détournables). Le second octet de drapeaux et le groupe ne sont pas encore interprétés.

## Fonctions utiles (`FeatureSet`)
Présentes sur les deux : `UnifiedBattery` (0x1004), `ReprogControlsV4` (0x1B04, v5 sur la 3S, v6 sur la MX4), `ChangeHost` (0x1814), `HostsInfo` (0x1815), `AdjustableDpi` (0x2201), `SmartShiftEnhanced` (0x2111), `HiResWheel` (0x2121), `WirelessDeviceStatus` (0x1D4B), `DeviceFirmwareInformation` (0x0003), `DeviceNameAndType` (0x0005).

Propres à la MX Master 4 :
- `ThumbWheel` (0x2150) : la molette du pouce, réglage (sens, vitesse), pas un bouton.
- `0x19B0` et `0x19C0` : inconnues du projet, présentes seulement sur la MX Master 4 ; candidates pour l'haptique (aucune source ne le confirme). `0x1701` aussi est inconnue. Aucune commande n'est envoyée à ces fonctions tant que leur description n'est pas établie.

Fonctions inconnues communes aux deux (`0x2250`, `0x2251`, `0x0020`, `0x0021`, `0x0007`, `0x0011`) : non explorées.

## À relever sur le poste
- `udevadm info -q path /dev/hidraw<N>` pour le nœud du Bolt (son chemin ne doit contenir qu'un niveau `0003:046D:C548.xxxx`) et `ls /sys/bus/usb/devices/*:1.2/0003:046D:C548.*/` : le Bolt n'a pas de nœud enfant aujourd'hui, la règle `DEVPATH` le garde si un pilote en créait.

## Fonctions écrites par `crates/mouse` (`link.rs`)
D'après le code de Solaar (`lib/logitech_receiver`, lu le 2026-11-19) ; **non encore essayées sur les souris** : à valider par Adrien avant de s'y fier. Numéros de fonction comme le protocole les compte (Solaar écrit `0x10` pour la fonction 1).

| Fonction | Lecture | Écriture |
|---|---|---|
| `DeviceFirmwareInformation` 0x0003 | f0 : octets 1 à 4 = identifiant de l'unité | |
| `UnifiedBattery` 0x1004 | f1 : % (0 = inconnu), niveau (8 plein, 4 bon, 2 bas, 1 critique), état | |
| `ReprogControlsV4` 0x1B04 | f2 (cid) : octet 2, bit 0 = détourné | f3 : cid (2 octets), drapeaux, remappage (2 octets) ; drapeaux : bit 0 détourner, bit 1 « bit 0 voulu » ; appuis reçus en notification, événement 0, quatre cid de 2 octets (0 = vide) |
| `AdjustableDpi` 0x2201 | f2 : capteur 0, DPI courant (2 octets) puis défaut | f3 : capteur 0, DPI (2 octets) |
| `SmartShiftEnhanced` 0x2111 | f1 : mode (1 libre, 2 cran), seuil (1 à 50, 255 jamais libre), couple | f2 : mode, seuil, couple (0 = inchangé pour les deux premiers) |
| `HiResWheel` 0x2121 | f1 : bit 0 détourné, bit 1 haute résolution, bit 2 inversé | f2 : cet octet |
| `ThumbWheel` 0x2150 | f1 : octet 0 bit 0 détourné, octet 1 bit 0 inversé | f2 : ces deux octets |
| `Haptic` 0x19B0 | f0 : masque des ondes (octets 4 à 7) ; f1 : actif, niveau 0 à 100 | f2 : actif + niveau, ou `0x00 0x32` pour couper ; f4 : jouer l'onde (1 octet) |

Identifiants d'ondes : `0x00` net, `0x01` amorti, `0x02` choc net, `0x03` choc amorti, `0x04` choc léger, `0x05` joyeuse, `0x06` sèche, `0x07` terminé, `0x08` carré, `0x09` vague, `0x0A` feu d'artifice, `0x0B` colère, `0x0C` toc-toc, `0x0D` grelot, `0x0E` sonnerie, `0x1B` murmure. Niveaux d'intensité retenus : subtil 25, faible 50, moyen 75, fort 100 (Solaar nomme 25, 50, 75, 100 « faible, moyen, fort, maximum »). Le contrôle `0x01A0` est nommé « Haptic » par Solaar. La vitesse de la molette du pouce n'existe pas côté souris (réglage du gestionnaire plus tard).

Garde-fous (revue sécurité du 2026-11-19) :
- Un `MouseLink` ne pilote que les modèles connus : à la connexion il lit le nom de l'appareil et refuse tout autre (un clavier Logitech a aussi `ReprogControlsV4`).
- Les boutons se désignent par contrôle du modèle (`MouseControlId`), jamais par identifiant brut ; un contrôle absent du modèle est refusé. Clics gauche et droit : jamais détournés (mais rendables, au cas où un autre programme les aurait pris). Le bit de persistance n'est jamais posé : le détournement est **temporaire**, la souris l'oublie à sa reconnexion (à réappliquer) et un bouton détourné reste mort tant qu'on ne le rend pas (`release_buttons` : au démarrage, à l'arrêt, à la reconnexion).
- Sensibilité (plage du contrat, pas celle annoncée par la souris), seuil de molette et intensité haptique vérifiés avant tout envoi.
- Écritures qui partagent un octet : lecture-modification-écriture qui garde les bits qu'elles ne possèdent pas (résolution et inversion de la molette ; inversion de la molette du pouce) ; une lecture vide ou courte arrête l'écriture.
- Les appuis détournés sont attribués à la souris qui les a envoyés (un récepteur en porte plusieurs, aux mêmes index de fonction) ; les trames de notification mises de côté pendant une requête sont bornées (256) et comptées quand on en jette : alors recommencer le suivi des boutons.
- Limites connues : version des fonctions non vérifiée, identifiant logiciel fixe (`0x0B`) : ne pas lancer Solaar, logiops ou Options+ en même temps (ils se disputeraient aussi les détournements).

## Validation sur le matériel (à faire par Adrien, jamais faite)
Tant que ce tableau est vide, les écritures sont « non validées sur le matériel ». Les tests passent sur un faux appareil qui reprend les mêmes hypothèses que le code (mêmes numéros de fonction, mêmes octets, issus de Solaar) : ils prouvent la cohérence, pas que les souris sont d'accord. Pour chaque point : date, souris, version du micrologiciel, trame observée (`--trace`), résultat.
1. Relever l'état avant toute écriture (sensibilité, molette, drapeaux, haptique) avec les seules lectures.
2. Sensibilité : 800, 1600, 4000, 8000, relire ; le pointeur change-t-il ? la valeur survit-elle à un redémarrage de la souris ?
3. Molette : cran, libre, automatique aux seuils 1, 25, 50 ; les octets de mode (1 libre, 2 cran) ; le couple ne bouge pas ; que fait 255, que fait 0 en mode libre.
4. Défilement : combinaisons résolution et inversion ; sens de chaque bit.
5. Molette du pouce (MX Master 4) : inversion, sens du défilement horizontal.
6. Détournement : chaque bouton (`0x52`, `0x53`, `0x56`, `0xC3`, `0xC4`, `0x01A0`), le bouton natif se tait-il, les appuis arrivent-ils, le rendre le restitue-t-il ; confirmer que `0x01A0` est le panneau haptique ; format réel d'une notification avec 1, 2, 3, 4 puis 5 boutons tenus.
7. Le détournement est-il perdu à la coupure, à la veille, au changement d'hôte Easy-Switch (attendu : oui) ?
8. Haptique (MX Master 4) : le masque des ondes (comparé à `0x0800_7FFF`), chaque onde annoncée, l'identifiant `0x1B`, ce que fait « couper » (niveau gardé à 50), la persistance après redémarrage, les niveaux 25/50/75/100 sont-ils ceux de Logitech (subtil, faible, moyen, fort) ?
9. Récepteur Logi Bolt, USB direct, Bluetooth : mêmes index et mêmes trames ?
10. Robustesse : première écriture après une longue inactivité (le délai de 500 ms suffit-il ?), débranchement en cours d'écriture.
11. Cohabitation avec Solaar ou Options+ ouverts.
12. Retour arrière : rendre tous les boutons, restaurer l'état relevé en 1.

## Haptique : ce qui est connu
Quatre niveaux d'intensité côté Logitech (subtil, faible, moyen, fort), actifs par défaut ; la maquette et le contrat en gardent quatre plus « désactivé ».
