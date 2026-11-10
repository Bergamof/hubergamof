# Image de construction et de test du paquet Arch/Manjaro (makepkg, pacman).
# L'utilisateur « builder » existe parce que makepkg refuse de tourner en root ;
# le test d'installation relance l'image en root (--user root).
FROM archlinux:base-devel@sha256:2de80105abb49f813a85393c6758a4334deb6bcec4a2411bd2f059c0bee6ee4c
RUN useradd --create-home --shell /bin/bash builder
USER builder
WORKDIR /home/builder
