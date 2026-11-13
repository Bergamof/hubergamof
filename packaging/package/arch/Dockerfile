# Image de construction et de test du paquet Arch/Manjaro (makepkg, pacman).
# L'utilisateur « builder » existe parce que makepkg refuse de tourner en root ;
# le test d'installation relance l'image en root (--user root).
FROM archlinux:base-devel@sha256:fa2d4ce3310ddd90a4208e5168be8ad367da5129af1245f38fd2d803bcbe3239
RUN useradd --create-home --shell /bin/bash builder
USER builder
WORKDIR /home/builder
