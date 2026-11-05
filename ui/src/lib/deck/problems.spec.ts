import { describe, expect, it } from 'vitest';
import type { ErrorCode } from '#lib/generated/index.js';
import { problemOf } from './problems.js';

/** Every code, `satisfies` failing as soon as the contract gains or loses one. */
const KEEPS_DETAILS = {
	invalidMessage: true,
	unknownDevice: false,
	invalidConfig: true,
	staleRevision: false,
	invalidInput: true,
	inputRefused: true,
	invalidImage: true,
	invalidArchive: true,
	internal: true
} as const satisfies Record<ErrorCode, boolean>;

describe('problemOf', () => {
	it.each(Object.entries(KEEPS_DETAILS) as [ErrorCode, boolean][])(
		'explains %s in French, with the details of the manager when useful',
		(code, keepsDetails) => {
			const problem = problemOf(code, 'english details');
			expect(problem.text).not.toContain('english details');
			expect(problem.text.length).toBeGreaterThan(10);
			expect(problem.detail).toBe(keepsDetails ? 'english details' : null);
		}
	);
});
