#!/usr/bin/env bash
# Impose un nom de branche parlant : `[claude/]<type>(/|-)<mots-en-kebab-case>`.
#
# - type : feat, fix, chore, docs, refactor, test, ci, perf ou build ;
# - au moins deux mots après le type (`fix/bug` ou `feat/update` ne disent rien) ;
# - préfixe `claude/` imposé par l'outil conservé, mais jamais le nom aléatoire
#   qu'il génère (`claude/youthful-dijkstra-nvx2dj`) : renommer la branche
#   (`git branch -m <nom>`) avant le premier commit.
# `main` et les branches de Dependabot sont admises.
#
# Usage : check-branch-name.sh [nom]   (défaut : branche courante)
set -euo pipefail

branch="${1:-$(git branch --show-current)}"
types='feat|fix|chore|docs|refactor|test|ci|perf|build'

[[ "$branch" == main || "$branch" == dependabot/* ]] && { echo "Branche « $branch » admise."; exit 0; }

if [[ "$branch" =~ ^(claude/)?($types)[/-][a-z0-9]+(-[a-z0-9]+)+$ ]]; then
	echo "Nom de branche « $branch » conforme."
	exit 0
fi

cat >&2 <<MESSAGE
Nom de branche « $branch » non conforme : attendu [claude/]<type>(/|-)<au-moins-deux-mots-en-kebab-case>.
Types : ${types//|/, }. Exemples : fix/node-minimum-version, claude/feat-clock-seconds.
Renommer : git branch -m <nouveau-nom>  (puis pousser la nouvelle branche).
MESSAGE
exit 1
