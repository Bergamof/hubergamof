import { describe, expect, it } from 'vitest';
import { managerSocketUrl } from './socket-url.js';

describe('managerSocketUrl', () => {
	it('uses ws: on a plain HTTP page, keeping host and port', () => {
		expect(managerSocketUrl({ protocol: 'http:', host: '127.0.0.1:7878' })).toBe(
			'ws://127.0.0.1:7878/ws'
		);
	});

	it('uses wss: on an HTTPS page', () => {
		expect(managerSocketUrl({ protocol: 'https:', host: 'deck.example' })).toBe(
			'wss://deck.example/ws'
		);
	});
});
