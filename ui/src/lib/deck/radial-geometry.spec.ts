import { describe, expect, it } from 'vitest';
import fixture from './radial-geometry.fixture.json';
import { RING, petalShapes } from './radial-geometry.js';

describe('petalShapes', () => {
	it('puts the first petal on the top and the next ones clockwise', () => {
		const [first, second, third, fourth] = petalShapes(4).map((shape) => shape.centre);
		const middle = (RING.inner + RING.outer) / 2;
		expect(first.x).toBeCloseTo(0);
		expect(first.y).toBeCloseTo(-middle);
		expect(second.x).toBeCloseTo(middle);
		expect(second.y).toBeCloseTo(0);
		expect(third.y).toBeCloseTo(middle);
		expect(fourth.x).toBeCloseTo(-middle);
	});

	it('draws one closed path per petal, the sides a gap apart', () => {
		const shapes = petalShapes(3);
		expect(shapes).toHaveLength(3);
		for (const shape of shapes) {
			expect(shape.path.startsWith('M ')).toBe(true);
			expect(shape.path.endsWith('Z')).toBe(true);
			expect(shape.path).not.toContain('NaN');
		}
		// The top petal of a menu of two is cut on the horizontal: its sides are pulled half a gap
		// up into the petal, away from the middle line.
		const [top] = petalShapes(2);
		const [, , y] = top.path.split(' ');
		expect(Number(y)).toBeCloseTo(-RING.gap / 2, 5);
	});

	it('draws a single entry as a whole ring with a hole', () => {
		const [only] = petalShapes(1);
		expect(only.path.match(/Z/g)).toHaveLength(2);
		expect(only.centre.y).toBeCloseTo(-(RING.inner + RING.outer) / 2);
	});
});

/**
 * The fixture is written by the Rust test of `radial/geometry.rs` from the geometry of the real menu: the
 * preview must draw the same corners.
 */
describe('petalShapes against the geometry of the real menu', () => {
	it('has the ring of the menu', () => {
		expect(RING).toEqual(fixture.ring);
	});

	it.each(fixture.menus)(
		'draws the $count petals of the menu with its corners',
		({ count, outlines }) => {
			const shapes = petalShapes(count);
			expect(shapes).toHaveLength(outlines.length);
			shapes.forEach((shape, index) => {
				// The path: M inner_start L outer_start A r r 0 0 1 outer_end L inner_end A r r 0 0 0 inner_start Z
				const token = shape.path.split(' ').map(Number);
				const found = {
					innerStart: [token[1], token[2]],
					outerStart: [token[4], token[5]],
					outerEnd: [token[12], token[13]],
					innerEnd: [token[15], token[16]]
				};
				for (const [name, expected] of Object.entries(outlines[index])) {
					const [x, y] = found[name as keyof typeof found];
					expect(x, `${count}/${index}/${name} x`).toBeCloseTo(expected[0], 4);
					expect(y, `${count}/${index}/${name} y`).toBeCloseTo(expected[1], 4);
				}
			});
		}
	);
});
