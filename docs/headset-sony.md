# Casque Sony WH-1000XM4 : protocole et relevés

**Statut (2026-11-24) : vérifié sur un vrai WH-1000XM4 pour le son ambiant, la batterie et la lecture de l'égaliseur** (relevés d'Adrien, ci-dessous). Les octets viennent à l'origine de projets tiers (Gadgetbridge, SonyHeadphonesClient, mdr-protocol) qui ne couvrent le XM4 que partiellement. Le crate `hubergamof-headset` et la commande `hubergamof headset-test` servent aux relevés ; la suite (contrat, gestionnaire, interface, module) s'appuie dessus. Décisions : `docs/decisions.md`, 2026-11-24.

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
| Fonction | Demande | Réponse / notification | État |
|---|---|---|---|
| Info protocole | `00 00` | `01 00 70 00` (version 0x7000) | **relevé** |
| Son ambiant | GET `66 02` ; SET `68 02 …` (8 octets) | `67` / `69`, 8 octets : effet, capacité vent, mode, ?, voix, niveau 0–20 | **relevé**, SET vérifié (`68 02 11 02 00 01 00 05` → `69 02 01 02 00 01 00 05`) |
| Batterie | `10 00` | `11 00 niveau charge` | **relevé** (`11 00 3C 00` = 60 %) |
| Égaliseur | GET `56 01` ; SET `58 01 <préréglage> 00` ou `58 01 FF 06 <6 valeurs + 10>` | `57` / `59`, 10 octets : préréglage, nombre de valeurs (`06`), clear bass, 5 bandes (valeur + 10) | lecture **relevée** (`57 01 17 06 00 0E 0D 0B 0C 00` = Speech) ; SET **à vérifier** |
| Appareils connectés | `24` | `25 31 01 00` | **sens inconnu** (aucune source) |
| Infos d'appareils, « concierge » | `1C`, `28` | inconnue | à sonder |

Le XM4 annonce **spontanément** un changement du son ambiant (`69`) : bouton NC/AMB du casque, appli, autre client. Les notifications ne sont émises qu'après la demande correspondante, donc l'application demande l'ambiance, la batterie et l'égaliseur à la connexion.

Octet de mode du son ambiant : sans capacité vent, `00` ambiant, `01` réduction de bruit ; avec capacité vent (**c'est le cas du XM4**, octet 3 = `02`), `00` ambiant, `01` vent, `02` réduction de bruit. Effet `00` = désactivé. Seules des valeurs fermées du contrat atteignent le casque.

Préréglages de l'égaliseur (table de Gadgetbridge) : `00` désactivé, `10` bright, `11` excited, `12` mellow, `13` relaxed, `14` vocal, `15` treble boost, `16` bass boost, `17` speech (**vu sur le XM4**), `A0` manuel, `A1`/`A2` personnalisés 1 et 2. Bandes, dans l'ordre : 400 Hz, 1 kHz, 2,5 kHz, 6,3 kHz, 16 kHz ; l'octet 4 est le clear bass. Sous Speech, clear bass et 16 kHz valent −10 : valeur réelle ou sentinelle, à vérifier.

## Un seul client à la fois
Quand l'appli Sony d'un téléphone se connecte, le casque envoie `A5 01 01 02` (sens inconnu ; `PLAY_NTFY_STATUS` dans mdr) puis **coupe la connexion RFCOMM** (`Connection reset by peer`) : un seul client de réglages à la fois. Nos trames avaient été acquittées, ce n'est pas un défaut d'ACK. Conséquence retenue (décision d'Adrien) : connexion permanente, interrupteur « Libérer », et après une coupure par l'autre appli **pas de nouvelle tentative**, un bouton « Reprendre la connexion ». À relever : une reconnexion de HuBergamof coupe-t-elle à son tour l'appli Android ?

## Ce qu'il reste à relever
1. Égaliseur : `equalizer preset bass-boost` puis `info` lit-il le préréglage `16` ? `equalizer manual` est-il pris en compte et relu à l'identique (clear bass, ordre des bandes) ? Retour à `speech` ensuite.
2. Appareils connectés (multipoint) : le sens de `25 31 01 00`, avec un second appareil connecté puis déconnecté (`probe connection-status`, puis `--arg 00`, `--arg 01`…), et `device-info`, `concierge`. Sinon : abandon, repli sur l'état de connexion vu par BlueZ.
3. Reconnexion de HuBergamof pendant que l'appli Android est connectée.
4. Le bouton CUSTOM est abandonné (aucun message d'appui connu).

## Faire les relevés
Casque appairé et connecté, application Sony du téléphone fermée (ou Bluetooth du téléphone coupé), service HuBergamof arrêté pour ne pas se disputer le canal (le crate n'est pas encore branché au service) :
```bash
hubergamof headset-test list
hubergamof headset-test info
hubergamof headset-test watch --seconds 120
hubergamof headset-test ambient ambient --level 5
hubergamof headset-test equalizer preset bass-boost
hubergamof headset-test equalizer manual --bass 0 --bands=0,0,0,0,0
hubergamof headset-test probe connection-status --arg 00
```
`watch` : changer pendant l'écoute le mode depuis le casque. Coller la sortie complète (lignes `->` envoyées, `<-` reçues, lecture en dessous). Les seules commandes qui écrivent sont `ambient` et `equalizer` : elles relisent aussitôt le résultat, et se défont par la même commande (`equalizer preset speech` rétablit le préréglage vu à l'origine). `probe` n'accepte des octets d'argument que pour `connection-status`, `device-info` et `concierge` (4 au plus, en hexadécimal) ; les autres demandes ont une charge utile connue.
