/**
 * The petals of the radial menu as the manager draws them (`radial/geometry.rs`, `radial/paint.rs`): a ring cut
 * into as many petals as entries, the first one centred on the top and the others following clockwise, with
 * parallel sides a gap apart. Pixels, y downwards, the menu centred on (0, 0).
 */

export const RING = { inner: 44, outer: 150, gap: 8 } as const;
/** Icon and label as the menu draws them. */
export const ICON_SIZE = 36;
export const LABEL_SIZE = 13;
export const LABEL_GAP = 6;
/** Room around the ring in a drawing. */
export const MARGIN = 8;

export interface Point {
	x: number;
	y: number;
}

export interface PetalShape {
	/** The `d` of the petal's path. */
	path: string;
	/** Middle of the petal, where its icon and label go. */
	centre: Point;
}

const FULL_TURN = 2 * Math.PI;

function circle(radius: number): string {
	return `M ${radius} 0 A ${radius} ${radius} 0 1 1 ${-radius} 0 A ${radius} ${radius} 0 1 1 ${radius} 0 Z`;
}

/** A corner of a petal side: `along` the side from the middle, pulled `gap / 2` into the petal. */
function corner(angle: number, radius: number, towardsClockwise: boolean): Point {
	const halfGap = RING.gap / 2;
	const along = Math.sqrt(Math.max(radius * radius - halfGap * halfGap, 0));
	const [sin, cos] = [Math.sin(angle), Math.cos(angle)];
	const sign = towardsClockwise ? 1 : -1;
	return {
		x: along * sin + sign * halfGap * cos,
		y: -along * cos + sign * halfGap * sin
	};
}

function petalPath(index: number, count: number): string {
	const step = FULL_TURN / count;
	const start = (index - 0.5) * step;
	const end = start + step;
	const innerStart = corner(start, RING.inner, true);
	const outerStart = corner(start, RING.outer, true);
	const outerEnd = corner(end, RING.outer, false);
	const innerEnd = corner(end, RING.inner, false);
	return (
		`M ${innerStart.x} ${innerStart.y} L ${outerStart.x} ${outerStart.y} ` +
		`A ${RING.outer} ${RING.outer} 0 0 1 ${outerEnd.x} ${outerEnd.y} ` +
		`L ${innerEnd.x} ${innerEnd.y} ` +
		`A ${RING.inner} ${RING.inner} 0 0 0 ${innerStart.x} ${innerStart.y} Z`
	);
}

/** The petals of a menu with `count` entries (a single one is the whole ring). */
export function petalShapes(count: number): PetalShape[] {
	const middleRadius = (RING.inner + RING.outer) / 2;
	return Array.from({ length: count }, (_, index) => {
		const angle = (index / count) * FULL_TURN;
		return {
			path: count < 2 ? `${circle(RING.outer)} ${circle(RING.inner)}` : petalPath(index, count),
			centre: { x: middleRadius * Math.sin(angle), y: -middleRadius * Math.cos(angle) }
		};
	});
}
