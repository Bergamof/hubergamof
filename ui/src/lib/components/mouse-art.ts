// Schémas des souris pour la page de configuration : dessins vectoriels (SVG détourés, sans fond) tirés des
// images de la maquette (usage personnel, à revoir si le dépôt devient public, comme les logos des modules).
// Chaque vue est un fichier, chargé seulement quand la souris s'affiche. Repères et zones cliquables sont placés
// à la main dans le repère du fichier (son `viewBox`) ; les positions sont des estimations, à valider sur chaque souris.
import type { MouseControlId, MouseModel } from '#lib/generated/index.js';
import anywhereSide from '#lib/assets/mouse/anywhere-side.svg';
import anywhereTop from '#lib/assets/mouse/anywhere-top.svg';
import masterSide from '#lib/assets/mouse/master-side.svg';
import masterTop from '#lib/assets/mouse/master-top.svg';

/** Zone cliquable d'un contrôle sur un schéma. */
export interface HitShape {
	id: MouseControlId;
	/** Élément SVG (`rect`, `path`, `circle`…). */
	tag: string;
	attrs: Record<string, string>;
}

/** Un point de vue sur la souris. */
export interface MouseView {
	id: 'top' | 'side';
	caption: string;
	/** Adresse du dessin (fichier SVG sans fond). */
	src: string;
	/** Taille du dessin, dans le repère de son `viewBox`. */
	size: { width: number; height: number };
	/** Largeur d'affichage, en pixels. */
	width: number;
	hits: HitShape[];
}

/** Où se trouve un contrôle, pour le repère et pour la description. */
export interface ControlSpot {
	id: MouseControlId;
	/** Numéro du repère. */
	number: number;
	/** Nom du contrôle sur ce modèle. */
	label: string;
	/** Où le trouver sur la souris. */
	where: string;
	view: MouseView['id'];
	x: number;
	y: number;
}

export interface MouseArt {
	views: MouseView[];
	spots: ControlSpot[];
}

/** Les repères gardent la même taille à l'écran (en pixels) quelle que soit la vue : leur échelle suit celle du dessin. */
export function markerScale(view: MouseView): number {
	return view.size.width / view.width;
}

export const MOUSE_ART: Record<MouseModel, MouseArt> = {
	mxMaster4: {
		views: [
			{
				id: 'top',
				caption: 'Vue de dessus',
				src: masterTop,
				size: { width: 738, height: 1048 },
				width: 280,
				hits: [
					{
						id: 'left',
						tag: 'path',
						attrs: { d: 'M215 105L300 42L435 12V100H375V400L310 425L190 470V130Z' }
					},
					{
						id: 'right',
						tag: 'path',
						attrs: { d: 'M435 12L560 42L650 90L670 130V462L500 405V100H435Z' }
					},
					{
						id: 'hapticPanel',
						tag: 'path',
						attrs: {
							d: 'M30 480L70 445L150 410L165 600L215 720L235 770L185 830L120 800L50 740L15 620Z'
						}
					},
					{
						id: 'wheelClick',
						tag: 'rect',
						attrs: { x: '400', y: '138', width: '72', height: '150', rx: '8' }
					},
					{
						id: 'wheelMode',
						tag: 'rect',
						attrs: { x: '408', y: '357', width: '56', height: '62', rx: '10' }
					}
				]
			},
			{
				id: 'side',
				caption: 'Côté pouce (face gauche)',
				src: masterSide,
				size: { width: 1168, height: 538 },
				width: 440,
				hits: [
					{
						id: 'thumbWheel',
						tag: 'rect',
						attrs: {
							x: '172',
							y: '122',
							width: '128',
							height: '90',
							rx: '40',
							transform: 'rotate(-20 236 167)'
						}
					},
					{
						id: 'forward',
						tag: 'rect',
						attrs: {
							x: '425',
							y: '266',
							width: '112',
							height: '36',
							rx: '18',
							transform: 'rotate(-17 481 284)'
						}
					},
					{
						id: 'back',
						tag: 'rect',
						attrs: {
							x: '545',
							y: '224',
							width: '128',
							height: '36',
							rx: '18',
							transform: 'rotate(-17 609 242)'
						}
					},
					{ id: 'gesture', tag: 'circle', attrs: { cx: '596', cy: '366', r: '44' } }
				]
			}
		],
		spots: [
			{
				id: 'left',
				number: 1,
				label: 'Clic gauche',
				where: 'Bouton gauche',
				view: 'top',
				x: 300,
				y: 230
			},
			{
				id: 'right',
				number: 2,
				label: 'Clic droit',
				where: 'Bouton droit',
				view: 'top',
				x: 575,
				y: 230
			},
			{
				id: 'wheelClick',
				number: 3,
				label: 'Molette (clic)',
				where: 'Appui sur la molette',
				view: 'top',
				x: 436,
				y: 213
			},
			{
				id: 'wheelMode',
				number: 4,
				label: 'Bouton de molette',
				where: 'Petit bouton derrière la molette',
				view: 'top',
				x: 436,
				y: 388
			},
			{
				id: 'back',
				number: 5,
				label: 'Précédent',
				where: 'Côté pouce, bouton arrière',
				view: 'side',
				x: 609,
				y: 242
			},
			{
				id: 'forward',
				number: 6,
				label: 'Suivant',
				where: 'Côté pouce, bouton avant',
				view: 'side',
				x: 481,
				y: 284
			},
			{
				id: 'gesture',
				number: 7,
				label: 'Bouton de geste',
				where: 'Côté pouce, sur le repose-pouce',
				view: 'side',
				x: 596,
				y: 366
			},
			{
				id: 'thumbWheel',
				number: 8,
				label: 'Molette du pouce',
				where: 'Au-dessus de Précédent et Suivant, rotation',
				view: 'side',
				x: 236,
				y: 167
			},
			{
				id: 'hapticPanel',
				number: 9,
				label: 'Panneau haptique',
				where: 'Repose-pouce, zone tactile',
				view: 'top',
				x: 95,
				y: 620
			}
		]
	},
	mxAnywhere3S: {
		views: [
			{
				id: 'top',
				caption: 'Vue de dessus',
				src: anywhereTop,
				size: { width: 582, height: 886 },
				width: 230,
				hits: [
					{
						id: 'left',
						tag: 'path',
						attrs: { d: 'M70 120L110 65L200 38L292 30V600H70L55 300Z' }
					},
					{
						id: 'right',
						tag: 'path',
						attrs: { d: 'M514 120L474 65L384 38L292 30V600H514L529 300Z' }
					},
					{
						id: 'wheelClick',
						tag: 'rect',
						attrs: { x: '255', y: '115', width: '72', height: '142', rx: '8' }
					},
					{
						id: 'wheelMode',
						tag: 'rect',
						attrs: { x: '268', y: '297', width: '42', height: '78', rx: '20' }
					}
				]
			},
			{
				id: 'side',
				caption: 'Côté pouce (face gauche)',
				src: anywhereSide,
				size: { width: 978, height: 354 },
				width: 400,
				hits: [
					{
						id: 'forward',
						tag: 'rect',
						attrs: { x: '352', y: '140', width: '140', height: '46', rx: '22' }
					},
					{
						id: 'back',
						tag: 'rect',
						attrs: {
							x: '502',
							y: '110',
							width: '128',
							height: '46',
							rx: '14',
							transform: 'rotate(-3 566 133)'
						}
					}
				]
			}
		],
		spots: [
			{
				id: 'left',
				number: 1,
				label: 'Clic gauche',
				where: 'Bouton gauche',
				view: 'top',
				x: 170,
				y: 300
			},
			{
				id: 'right',
				number: 2,
				label: 'Clic droit',
				where: 'Bouton droit',
				view: 'top',
				x: 415,
				y: 300
			},
			{
				id: 'wheelClick',
				number: 3,
				label: 'Molette (clic)',
				where: 'Appui sur la molette',
				view: 'top',
				x: 291,
				y: 186
			},
			{
				id: 'wheelMode',
				number: 4,
				label: 'Bouton du dessus',
				where: 'Bouton ovale sous la molette',
				view: 'top',
				x: 289,
				y: 337
			},
			{
				id: 'back',
				number: 5,
				label: 'Précédent',
				where: 'Côté pouce, bouton arrière',
				view: 'side',
				x: 566,
				y: 133
			},
			{
				id: 'forward',
				number: 6,
				label: 'Suivant',
				where: 'Côté pouce, bouton avant',
				view: 'side',
				x: 422,
				y: 163
			}
		]
	}
};
