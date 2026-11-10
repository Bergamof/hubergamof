#!/usr/bin/env bash
# Porte d'entrée de la release : la CI du commit à publier doit être terminée
# et verte. Attend si elle est en cours, échoue si elle est en erreur, annulée
# ou introuvable.
#
# Usage : wait-for-ci.sh <sha>            (attend, avec GH_TOKEN et GITHUB_REPOSITORY)
#         wait-for-ci.sh --verdict        (lit les runs en JSON sur l'entrée standard)
#
# --verdict affiche : success | pending | failure | missing. Le run retenu est
# le plus récent du commit, quel que soit l'événement (push, PR, planification).
# Codes (attente) : 0 CI verte, 1 CI en échec ou introuvable, 2 usage.
set -euo pipefail

readonly CI_WORKFLOW="ci.yml"
readonly POLL_SECONDS="${WAIT_FOR_CI_POLL_SECONDS:-20}"
# Le run du push peut ne pas encore exister juste après le merge.
readonly APPEARANCE_GRACE_SECONDS="${WAIT_FOR_CI_GRACE_SECONDS:-180}"
readonly MAX_WAIT_SECONDS="${WAIT_FOR_CI_MAX_SECONDS:-3600}"

verdict() {
	jq -r '
		(.workflow_runs // []) | sort_by(.created_at) | last
		| if . == null then "missing"
		  elif .status != "completed" then "pending"
		  elif .conclusion == "success" then "success"
		  else "failure" end
	'
}

if [[ "${1:-}" == --verdict ]]; then
	verdict
	exit 0
fi

if (($# != 1)); then
	echo "Usage : $(basename "$0") <sha> | --verdict" >&2
	exit 2
fi
sha="$1"
repository="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY manquant}"

started="$SECONDS"
while true; do
	runs="$(gh api "repos/$repository/actions/workflows/$CI_WORKFLOW/runs?head_sha=$sha&per_page=100")"
	result="$(verdict <<<"$runs")"
	elapsed=$((SECONDS - started))
	case "$result" in
	success)
		echo "CI verte sur $sha."
		exit 0
		;;
	failure)
		echo "La CI du commit $sha est en échec (ou annulée) : release refusée." >&2
		exit 1
		;;
	missing)
		if ((elapsed >= APPEARANCE_GRACE_SECONDS)); then
			echo "Aucun run de CI pour $sha : release refusée." >&2
			exit 1
		fi
		echo "Pas encore de run de CI pour $sha, nouvelle tentative…"
		;;
	pending) echo "CI en cours sur $sha, attente… (${elapsed}s)" ;;
	esac
	if ((elapsed >= MAX_WAIT_SECONDS)); then
		echo "Délai d'attente de la CI dépassé (${MAX_WAIT_SECONDS}s)." >&2
		exit 1
	fi
	sleep "$POLL_SECONDS"
done
