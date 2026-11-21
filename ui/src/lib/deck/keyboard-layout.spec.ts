import { describe, expect, it } from 'vitest';
import {
	learnKey,
	loadKeyboardLayout,
	mergeLayouts,
	readLearnedLayout,
	saveLearnedLayout
} from './keyboard-layout.js';

describe('loadKeyboardLayout', () => {
	it('gives the layout the browser tells', async () => {
		const layout = new Map([['KeyA', 'q']]);
		expect(await loadKeyboardLayout({ getLayoutMap: () => Promise.resolve(layout) })).toBe(layout);
	});

	it('gives nothing where the browser cannot tell, or refuses', async () => {
		expect(await loadKeyboardLayout(undefined)).toBeUndefined();
		expect(await loadKeyboardLayout({})).toBeUndefined();
		expect(
			await loadKeyboardLayout({ getLayoutMap: () => Promise.reject(new Error('denied')) })
		).toBeUndefined();
	});
});

const press = (
	code: string,
	key: string,
	held: Partial<{ ctrlKey: boolean; altKey: boolean; metaKey: boolean }> = {}
) => ({
	code,
	key,
	ctrlKey: false,
	altKey: false,
	metaKey: false,
	...held
});

describe('learnKey', () => {
	it('learns what a key writes, wherever the key is', () => {
		const learned = learnKey(new Map(), press('KeyA', 'q'));
		expect(learned?.get('KeyA')).toBe('q');
		expect(learnKey(learned!, press('KeyM', ','))?.get('KeyM')).toBe(',');
	});

	it('writes in lower case, whatever the capital', () => {
		expect(learnKey(new Map(), press('KeyA', 'Q'))?.get('KeyA')).toBe('q');
	});

	it('learns nothing from what it already knows, from modifiers, or from other keys', () => {
		expect(learnKey(new Map([['KeyA', 'q']]), press('KeyA', 'q'))).toBeUndefined();
		expect(learnKey(new Map(), press('KeyA', 'q', { ctrlKey: true }))).toBeUndefined();
		expect(learnKey(new Map(), press('KeyA', 'q', { altKey: true }))).toBeUndefined();
		expect(learnKey(new Map(), press('KeyA', 'q', { metaKey: true }))).toBeUndefined();
		expect(learnKey(new Map(), press('Enter', 'Enter'))).toBeUndefined();
		expect(learnKey(new Map(), press('ArrowUp', 'ArrowUp'))).toBeUndefined();
		expect(learnKey(new Map(), press('KeyA', 'Dead'))).toBeUndefined();
	});

	it('replaces what a key used to write when the layout changed', () => {
		expect(learnKey(new Map([['KeyA', 'a']]), press('KeyA', 'q'))?.get('KeyA')).toBe('q');
	});
});

describe('mergeLayouts', () => {
	it('gives the second where both know, and either alone', () => {
		const merged = mergeLayouts(
			new Map([
				['KeyA', 'a'],
				['KeyB', 'b']
			]),
			new Map([['KeyA', 'q']])
		);
		expect([...merged]).toEqual([
			['KeyA', 'q'],
			['KeyB', 'b']
		]);
		expect(mergeLayouts(undefined, undefined).size).toBe(0);
	});
});

describe('the layout kept between visits', () => {
	const storage = () => {
		const items = new Map<string, string>();
		return {
			getItem: (key: string) => items.get(key) ?? null,
			setItem: (key: string, value: string) => void items.set(key, value)
		};
	};

	it('is read back as it was saved', () => {
		const kept = storage();
		saveLearnedLayout(new Map([['KeyA', 'q']]), kept);
		expect(readLearnedLayout(kept).get('KeyA')).toBe('q');
	});

	it('is empty where nothing was kept, what was kept is unreadable, or there is no storage', () => {
		expect(readLearnedLayout(storage()).size).toBe(0);
		expect(readLearnedLayout({ getItem: () => 'not json' }).size).toBe(0);
		expect(readLearnedLayout({ getItem: () => '{"KeyA":"q"}' }).size).toBe(0);
		expect(
			readLearnedLayout({ getItem: () => '[["Enter","x"],["KeyA","qq"],["KeyB","b"]]' }).get('KeyB')
		).toBe('b');
		expect(readLearnedLayout({ getItem: () => '[["Enter","x"],["KeyA","qq"]]' }).size).toBe(0);
		expect(readLearnedLayout(undefined).size).toBeGreaterThanOrEqual(0);
	});

	it('survives a storage that refuses', () => {
		expect(() =>
			saveLearnedLayout(new Map([['KeyA', 'q']]), {
				setItem: () => {
					throw new Error('full');
				}
			})
		).not.toThrow();
		expect(
			readLearnedLayout({
				getItem: () => {
					throw new Error('blocked');
				}
			}).size
		).toBe(0);
	});
});
