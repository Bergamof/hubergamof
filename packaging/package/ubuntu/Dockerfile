# Image propre pour installer le .deb : Ubuntu 24.04, la plus ancienne version
# prise en charge (pactl 16). Le digest est rafraîchi par Dependabot.
# L'utilisateur par défaut est non-root (règle Trivy DS-0002) ; le test
# d'installation relance l'image en root (--user root), apt l'exigeant.
FROM ubuntu:26.04@sha256:6a06111c3b0a8600feadb270aec771dc239bf407b25625f93f44ab00e11f4d3c
RUN useradd --create-home --shell /bin/bash tester
USER tester
