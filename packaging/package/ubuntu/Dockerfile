# Image propre pour installer le .deb : Ubuntu 24.04, la plus ancienne version
# prise en charge (pactl 16). Le digest est rafraîchi par Dependabot.
# L'utilisateur par défaut est non-root (règle Trivy DS-0002) ; le test
# d'installation relance l'image en root (--user root), apt l'exigeant.
FROM ubuntu:24.04@sha256:9c7f200c07e3d4a03e7249d2c16bf66a51120fb876aa2242cf31451412543289
RUN useradd --create-home --shell /bin/bash tester
USER tester
