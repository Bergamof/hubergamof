import { describe, expect, it } from 'vitest';
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
