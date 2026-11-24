# Casque Sony WH-1000XM4 : protocole et relevés

**Statut (2026-11-24) : rien n'est encore vérifié sur un vrai XM4.** Les octets viennent de projets tiers (Gadgetbridge, SonyHeadphonesClient, mdr-protocol) qui ne couvrent le XM4 que partiellement. Le crate `hubergamof-headset` et la commande `hubergamof headset-test` existent pour faire ces vérifications ; la suite (contrat, gestionnaire, interface, module) attend les relevés. Décision : `docs/decisions.md`, 2026-11-24.

## Transport
- RFCOMM, service Bluetooth `96CC203E-5068-46AD-B32D-E316F5E069BA` (canal trouvé par SDP, donc par BlueZ).
- Le crate ne peut pas ouvrir de socket Bluetooth (`unsafe` interdit) : il exporte un objet D-Bus `org.bluez.Profile1` (`/com/hubergamof/headset`, rôle client), l'enregistre auprès de `ProfileManager1`, appelle `Device1.ConnectProfile(uuid)` et BlueZ lui remet le socket connecté (`NewConnection`). Aucune règle udev ; l'accès dépend de la politique D-Bus de BlueZ (session active), à vérifier sur Manjaro et Ubuntu.
- Un seul client à la fois sur ce canal : l'application Sony d'un téléphone connectée au casque peut le prendre.

## Trame (`crates/headset/src/frame.rs`)
`3E [type][séquence][longueur u32 BE][charge utile][somme] 3C`.
- Somme : somme 8 bits des octets avant elle, **avant** échappement.
- Échappement : `3C`, `3D`, `3E` du corps deviennent `3D` suivi de l'octet avec le bit `0x10` éteint (`3D 2C`, `3D 2D`, `3D 2E`).
- Acquittement : type `01`, charge vide, séquence inversée par rapport à la trame reçue. L'émetteur ne bascule sa séquence qu'après acquittement et renvoie la même trame sans réponse (3 essais, 1 s).
- Exemple : demande de l'état du son ambiant `66 02`, type `0C`, séquence 0 → `3E 0C 00 00 00 00 02 66 02 76 3C`.
- Type de données : `0C` supposé (le plus courant) ; `0E` et `00` s'essaient avec `--data-type`.

## Charges utiles (`payload.rs`)
| Fonction | Demande | Réponse / notification |
|---|---|---|
| Son ambiant | GET `66 02` ; SET `68 02 …` (8 octets) | `67` / `69`, 8 octets : effet, capacité vent, mode, ?, voix, niveau 0–20 |
| Batterie | `10 00` | `11` / `13` : `00 niveau charge` (un bloc) |
| Égaliseur | `56 01` | `57` / `59`, 10 octets : préréglage, clear bass, 5 bandes (valeur + 10) |
| Info protocole | `00 00` (à confirmer) | inconnue |
| Appareils connectés | `24` ou `1C` seuls (octets suivants inconnus) | inconnue |

Les notifications (`69`, `59`, `13`) ne sont émises qu'après la demande correspondante : l'application les demande toutes à la connexion.

Octet de mode du son ambiant : sans capacité vent, `00` ambiant, `01` réduction de bruit ; avec capacité vent, `00` ambiant, `01` vent, `02` réduction de bruit. Octet de capacité : `00` ou `02`. Seules des valeurs fermées du contrat atteignent le casque.

## Ce qu'il reste à relever
1. Le XM4 acquitte-t-il le type `0C` ? Que répond `protocol-info` ?
2. Capacité vent du XM4 (octet 3 de `67`) ; une demande de changement du son ambiant est-elle prise en compte et notifiée ?
3. Un appui physique sur le bouton NC/AMB envoie-t-il un `69` spontané ?
4. Codes des préréglages de l'égaliseur et ordre des bandes (changer le préréglage dans l'appli Android pendant `watch`).
5. Appareils connectés (multipoint) : octets de `24`/`1C`, notification quand un appareil se (dé)connecte. Sinon : abandon, repli sur l'état de connexion vu par BlueZ.
6. Le bouton CUSTOM est abandonné (aucun message d'appui connu).

## Faire les relevés
Casque appairé et connecté, application Sony du téléphone fermée (ou Bluetooth du téléphone coupé), service HuBergamof arrêté pour ne pas se disputer le canal (le crate n'est pas encore branché au service) :
```bash
hubergamof headset-test list
hubergamof headset-test info
hubergamof headset-test watch --seconds 120
hubergamof headset-test ambient ambient --level 5
hubergamof headset-test probe connection-status
```
`watch` : changer pendant l'écoute le mode, le préréglage ou les appareils connectés depuis le casque ou l'appli Android. Coller la sortie complète (lignes `->` envoyées, `<-` reçues, lecture en dessous) ; sans réponse au type `0C`, relancer avec `--data-type mdr2` puis `data`. `ambient` est la seule commande qui écrit : un changement de mode, remis comme avant par la même commande.
