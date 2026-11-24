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
- Type de données : `0C`, **vérifié sur le XM4** (les réponses reviennent en `0C`, sauf la liste des appareils en `0E`) ; `0E` et `00` s'essaient avec `--data-type`.

## Charges utiles (`payload.rs`)
| Fonction | Demande | Réponse / notification | État |
|---|---|---|---|
| Info protocole | `00 00` | `01 00 70 00` (version 0x7000) | **relevé** |
| Son ambiant | GET `66 02` ; SET `68 02 …` (8 octets) | `67` / `69`, 8 octets : effet, capacité vent, mode, ?, voix, niveau 0–20 | **relevé**, SET vérifié (`68 02 11 02 00 01 00 05` → `69 02 01 02 00 01 00 05`) |
| Batterie | `10 00` | `11 00 niveau charge` | **relevé** (`11 00 3C 00` = 60 %) |
| Égaliseur | GET `56 01` ; SET `58 01 <préréglage> 00` ou `58 01 FF 06 <6 valeurs + 10>` | `57` / `59`, 10 octets : préréglage, nombre de valeurs (`06`), clear bass, 5 bandes (valeur + 10) | lecture **relevée** (`57 01 17 06 00 0E 0D 0B 0C 00` = Speech) ; SET d'un **préréglage vérifié** (`58 01 16 00` → `57 01 16 06 11 0A 0A 0A 0A 0A` : clear bass +7 ; `58 01 17 00` répond aussitôt `59 …`) ; SET manuel **vérifié** (`58 01 FF 06 0A 0A 0A 0A 0A 0A` → `59 01 A0 06 0A 0A 0A 0A 0A 0A` : le préréglage devient `A0`, relu à l'identique) |
| État de connexion | `24` seul | `25 31 01 00` | **sens inconnu** ; `24 00` et `24 01` ne répondent rien |
| Appareils appairés | `38` (hypothèse : le GET voisin du `39`) | `39 01 N …` sur le type `0E` | format décodé (ci-dessous), requête **à vérifier** |
| Infos d'appareils, « concierge » | `1C`, `28` | inconnue | à sonder |

Le XM4 annonce **spontanément** un changement du son ambiant (`69`) : bouton NC/AMB du casque, appli, autre client. Les notifications ne sont émises qu'après la demande correspondante, donc l'application demande l'ambiance, la batterie et l'égaliseur à la connexion.

Octet de mode du son ambiant : sans capacité vent, `00` ambiant, `01` réduction de bruit ; avec capacité vent (**c'est le cas du XM4**, octet 3 = `02`), `00` ambiant, `01` vent, `02` réduction de bruit. Effet `00` = désactivé. Seules des valeurs fermées du contrat atteignent le casque.

Préréglages de l'égaliseur (table de Gadgetbridge) : `00` désactivé, `10` bright, `11` excited, `12` mellow, `13` relaxed, `14` vocal, `15` treble boost, `16` bass boost, `17` speech (**vu sur le XM4**), `A0` manuel, `A1`/`A2` personnalisés 1 et 2. Bandes, dans l'ordre : 400 Hz, 1 kHz, 2,5 kHz, 6,3 kHz, 16 kHz ; l'octet 4 est le clear bass. Sous Speech, clear bass et 16 kHz valent −10 : valeur réelle ou sentinelle, à vérifier.

## Liste des appareils appairés (`39`)
Message vu une fois, spontané, sur le type de données **`0E`** (second canal MDR), après un changement d'égaliseur. Les codes vont par quatre (GET, RET, SET, NTFY : `56/57/58/59`, `66/67/68/69`, `24/25/26/27`) : `39` serait le RET de `38`. Format, vérifié sur les octets reçus (cinq appareils) :
`39 01 N`, puis N fois `[adresse en ASCII, 17 octets][octet d'état][longueur du nom][nom]`, puis un octet final (`02` vu).
Octets d'état vus : `01`, `02`, `00`, `00`, `00` ; l'ordinateur qui parle au casque était à `00` alors qu'il était connecté : **sens inconnu**. Octet final : sens inconnu (le nombre d'appareils connectés ?). `decode` borne tout (8 appareils au plus, adresse vérifiée, nom nettoyé).

## Un seul client à la fois
Quand l'appli Sony d'un téléphone se connecte, le casque envoie `A5 01 01 02` (sens inconnu ; `PLAY_NTFY_STATUS` dans mdr) puis **coupe la connexion RFCOMM** (`Connection reset by peer`) : un seul client de réglages à la fois. Nos trames avaient été acquittées, ce n'est pas un défaut d'ACK. Relevé : quand HuBergamof se connecte, **c'est l'appli Android qui est coupée** (l'audio continue : un appel est resté en cours, seule la liaison de réglages tombe). Conséquence retenue (décision d'Adrien, 2026-11-24) : HuBergamof se connecte **à l'accès à la page de réglages** (jamais de lui-même au retour du casque) ; une coupure en cours d'usage n'est **pas retentée**, un bouton « Reprendre la connexion » la remplace ; un interrupteur « Libérer » ferme la connexion.

## Ce qu'il reste à relever
1. Liste des appareils : la requête (`hubergamof headset-test --data-type mdr2 probe paired-devices`, puis `… probe paired-devices --arg 01`) ; à défaut `hubergamof headset-test --data-type mdr2 watch --seconds 120` pendant qu'un téléphone se connecte puis se déconnecte au casque : quand `39` arrive, comment l'octet d'état change. (Les options `--data-type` et `--address` se placent avant ou après l'action.)
2. Égaliseur, facultatif : l'ordre des bandes par rapport aux fréquences (`equalizer manual --bass 6 --bands=1,2,3,4,5`, regarder les curseurs dans l'appli Sony, puis `equalizer preset speech`) ; les préréglages personnalisés `custom1`/`custom2`.
3. Le bouton CUSTOM est abandonné (aucun message d'appui connu).

**Vérifié sur le XM4** : `info` répond à toutes les demandes (correctif de `Link`), préréglages et bandes de l'égaliseur se règlent et se relisent.

## Réponses perdues (corrigé)
Les relevés montraient des réponses absentes (l'ambiance dans `info`, la batterie ailleurs). Cause, dans `Link::wait_for_ack` : un paquet RFCOMM qui portait l'ACK **et** la réponse derrière lui perdait la réponse. Toutes les trames d'un paquet sont maintenant gardées, dans l'ordre (tests avec `FakeHeadset::joining_reads`).

## Faire les relevés
Casque appairé et connecté, application Sony du téléphone fermée (ou Bluetooth du téléphone coupé), service HuBergamof arrêté pour ne pas se disputer le canal (le crate n'est pas encore branché au service) :
```bash
hubergamof headset-test list
hubergamof headset-test info
hubergamof headset-test watch --seconds 120
hubergamof headset-test ambient ambient --level 5
hubergamof headset-test equalizer preset bass-boost
hubergamof headset-test equalizer manual --bass 0 --bands=0,0,0,0,0
hubergamof headset-test --data-type mdr2 probe paired-devices
```
`watch` : changer pendant l'écoute le mode depuis le casque. Coller la sortie complète (lignes `->` envoyées, `<-` reçues, lecture en dessous). Les seules commandes qui écrivent sont `ambient` et `equalizer` : elles relisent aussitôt le résultat, et se défont par la même commande (`equalizer preset speech` rétablit le préréglage vu à l'origine). `probe` n'accepte des octets d'argument que pour `connection-status`, `device-info` et `concierge` (4 au plus, en hexadécimal) ; les autres demandes ont une charge utile connue.
