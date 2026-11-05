import type { ErrorCode } from '#lib/generated/index.js';

/** A message for the user: French text, and the manager's own details (in English), if any. */
export interface Problem {
	text: string;
	detail: string | null;
}

/** French explanation of a manager error, its English message as details. */
export function problemOf(code: ErrorCode, message: string): Problem {
	switch (code) {
		case 'staleRevision':
			return {
				text: 'La configuration a été modifiée ailleurs entre-temps : la version à jour est affichée, refaites la modification.',
				detail: null
			};
		case 'invalidConfig':
			return {
				text: 'Le gestionnaire a refusé la modification : vérifiez les réglages.',
				detail: message
			};
		case 'unknownDevice':
			return { text: 'Cet appareil n’est plus connecté.', detail: null };
		case 'invalidImage':
			return {
				text: 'Image refusée : elle doit être un PNG, un JPEG ou un SVG lisible.',
				detail: message
			};
		case 'invalidArchive':
			return {
				text: 'Fichier refusé : ce n’est pas un export HuBergamof valide.',
				detail: message
			};
		default:
			return { text: 'Le gestionnaire a signalé une erreur.', detail: message };
	}
}
