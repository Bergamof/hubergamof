import { describe, expect, it } from 'vitest';
import { parseServerMessage } from './server-message.js';

describe('parseServerMessage', () => {
	it('returns a message with a known type', () => {
		expect(parseServerMessage('{"type":"hello","version":"0.1.0"}')).toEqual({
			type: 'hello',
			version: '0.1.0'
		});
	});

	it.each([
		['non-string data', new ArrayBuffer(2)],
		['invalid JSON', 'not json'],
		['null', 'null'],
		['an array', '[{"type":"pong"}]'],
		['a non-string type', '{"type":1}'],
		['an unknown type', '{"type":"toString"}']
	])('rejects %s', (_label, raw) => {
		expect(parseServerMessage(raw)).toBeUndefined();
	});
});
