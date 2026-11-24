import { describe, expect, it } from 'vitest';
import type { HeadsetInfo, HeadsetState as Status } from '#lib/generated/index.js';
import { DeckState } from './deck-state.svelte.js';
import { FakeConnection } from './fake-connection.js';

const ADDRESS = 'AA:BB:CC:DD:EE:FF';
const OTHER = '11:22:33:44:55:66';
const headset = (address: string): HeadsetInfo => ({ address, name: `XM4 ${address}` });

const status = (link: Status['link'], extra: Partial<Status> = {}): Status => ({
	link,
	windReduction: true,
	...extra
});

function setup() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	return { connection, headsets: deck.headsets };
}

describe('HeadsetState', () => {
	it('lists the headphones without following them: following opens the link', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		expect(headsets.headsets).toEqual([headset(ADDRESS)]);
		expect(connection.sent).toEqual([]);
	});

	it('follows the first headphones when the page opens, and again on every list', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		headsets.enter();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS), headset(OTHER)] });
		expect(connection.sent).toEqual([
			{ type: 'selectHeadset', address: ADDRESS },
			{ type: 'selectHeadset', address: ADDRESS }
		]);
	});

	it('follows nothing while the page is closed, and forgets what they said', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		headsets.enter();
		connection.receive({ type: 'headsetState', address: ADDRESS, state: status('connected') });
		headsets.leave();
		expect(headsets.status).toBeUndefined();
		connection.sent.length = 0;
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		expect(connection.sent).toEqual([]);
		expect(headsets.setAmbient({ mode: 'off', level: 0, focusOnVoice: false })).toBe(false);
	});

	it('takes what the followed headphones say, not what others say', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS), headset(OTHER)] });
		headsets.enter();
		connection.receive({ type: 'headsetState', address: OTHER, state: status('cut') });
		expect(headsets.status).toBeUndefined();
		connection.receive({ type: 'headsetState', address: ADDRESS, state: status('connected') });
		expect(headsets.status?.link).toBe('connected');
	});

	it('moves to the first headphones when the followed ones leave, and forgets everything when none are left', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS), headset(OTHER)] });
		headsets.enter();
		connection.receive({ type: 'headsetState', address: ADDRESS, state: status('connected') });
		connection.receive({ type: 'headsets', headsets: [headset(OTHER)] });
		expect(connection.lastSent()).toEqual({ type: 'selectHeadset', address: OTHER });
		expect(headsets.status).toBeUndefined();
		connection.receive({ type: 'headsets', headsets: [] });
		expect(headsets.headset).toBeUndefined();
		expect(headsets.resume()).toBe(false);
	});

	it('sends the requests to the followed headphones', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		headsets.enter();
		const ambient = { mode: 'ambient', level: 7, focusOnVoice: true } as const;
		expect(headsets.setAmbient(ambient)).toBe(true);
		expect(headsets.setEqualizer({ type: 'preset', preset: 'speech' })).toBe(true);
		expect(headsets.release()).toBe(true);
		expect(headsets.resume()).toBe(true);
		expect(connection.sent.slice(1)).toEqual([
			{ type: 'setHeadsetAmbient', address: ADDRESS, ambient },
			{
				type: 'setHeadsetEqualizer',
				address: ADDRESS,
				equalizer: { type: 'preset', preset: 'speech' }
			},
			{ type: 'releaseHeadset', address: ADDRESS },
			{ type: 'resumeHeadset', address: ADDRESS }
		]);
	});

	it('forgets what they said when the connection is new, and follows them again with the list', () => {
		const { connection, headsets } = setup();
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		headsets.enter();
		connection.receive({ type: 'headsetState', address: ADDRESS, state: status('connected') });
		connection.receive({ type: 'hello', version: '1.0.0' });
		expect(headsets.status).toBeUndefined();
		connection.sent.length = 0;
		connection.receive({ type: 'headsets', headsets: [headset(ADDRESS)] });
		expect(connection.sent).toEqual([{ type: 'selectHeadset', address: ADDRESS }]);
	});
});
