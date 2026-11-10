# Image propre pour installer le .deb : Ubuntu 24.04, la plus ancienne version
# prise en charge (pactl 16). Le digest est rafraîchi par Dependabot.
# L'utilisateur par défaut est non-root (règle Trivy DS-0002) ; le test
# d'installation relance l'image en root (--user root), apt l'exigeant.
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
RUN useradd --create-home --shell /bin/bash tester
USER tester
