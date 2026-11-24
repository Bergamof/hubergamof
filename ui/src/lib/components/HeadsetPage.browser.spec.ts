import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { HeadsetState } from '#lib/generated/index.js';
import HeadsetPage from './HeadsetPage.svelte';

const ADDRESS = 'AA:BB:CC:DD:EE:FF';

const connectedState = (extra: Partial<HeadsetState> = {}): HeadsetState => ({
	link: 'connected',
	windReduction: true,
	battery: { percent: 80, charging: false },
	ambient: { mode: 'ambient', level: 10, focusOnVoice: false },
	equalizer: { preset: 'speech', clearBass: 0, bands: [0, 14, 13, 11, 12] },
	...extra
});

async function renderPage(state?: HeadsetState) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'headsets', headsets: [{ address: ADDRESS, name: 'WH-1000XM4' }] });
	await render(HeadsetPage, { deck });
	if (state) connection.receive({ type: 'headsetState', address: ADDRESS, state });
	return { connection };
}

describe('HeadsetPage', () => {
	it('says there are no headphones when none is connected', async () => {
		const connection = new FakeConnection();
		const deck = new DeckState(connection, { load: () => null, save: () => undefined });
		await render(HeadsetPage, { deck });
		await expect.element(page.getByText(/Aucun casque connecté/)).toBeInTheDocument();
	});

	it('follows the headphones as it opens, which opens the link', async () => {
		const { connection } = await renderPage();
		expect(connection.sent).toContainEqual({ type: 'selectHeadset', address: ADDRESS });
		await expect.element(page.getByRole('heading', { name: 'WH-1000XM4' })).toBeVisible();
		await expect.element(page.getByText('Pas encore connecté')).toBeVisible();
	});

	it('shows the battery and warns when it is low', async () => {
		await renderPage(connectedState());
		await expect.element(page.getByText('Batterie 80 %')).toBeVisible();
		await renderPage(connectedState({ battery: { percent: 12, charging: false } }));
		await expect.element(page.getByText('Batterie 12 %, faible')).toBeVisible();
	});

	it('changes the ambient mode and level, and the voice setting', async () => {
		const { connection } = await renderPage(connectedState());
		await page.getByRole('radio', { name: 'Réduction de bruit' }).click({ force: true });
		expect(connection.lastSent()).toEqual({
			type: 'setHeadsetAmbient',
			address: ADDRESS,
			ambient: { mode: 'noiseCancelling', level: 10, focusOnVoice: false }
		});
		await page.getByRole('switch', { name: 'Laisser passer les voix' }).click({ force: true });
		expect(connection.lastSent()).toEqual({
			type: 'setHeadsetAmbient',
			address: ADDRESS,
			ambient: { mode: 'ambient', level: 10, focusOnVoice: true }
		});
	});

	it('only offers the wind reduction when the headphones have it, and the level in ambient mode', async () => {
		await renderPage(
			connectedState({
				windReduction: false,
				ambient: { mode: 'noiseCancelling', level: 0, focusOnVoice: false }
			})
		);
		await expect
			.element(page.getByRole('radio', { name: 'Réduction du vent' }))
			.not.toBeInTheDocument();
		await expect
			.element(page.getByRole('slider', { name: 'Niveau du son ambiant' }))
			.not.toBeInTheDocument();
	});

	it('picks an equalizer preset and moves a band, which makes it manual', async () => {
		const { connection } = await renderPage(connectedState());
		await page.getByLabelText('Préréglage de l’égaliseur').selectOptions('bassBoost');
		expect(connection.lastSent()).toEqual({
			type: 'setHeadsetEqualizer',
			address: ADDRESS,
			equalizer: { type: 'preset', preset: 'bassBoost' }
		});
		await page.getByRole('slider', { name: 'Égaliseur : 1 kHz' }).fill('3');
		expect(connection.lastSent()).toEqual({
			type: 'setHeadsetEqualizer',
			address: ADDRESS,
			equalizer: { type: 'manual', clearBass: 0, bands: [0, 3, 13, 11, 12] }
		});
	});

	it('releases the link, and takes it back on request after a release or a cut', async () => {
		const { connection } = await renderPage(connectedState());
		await page.getByRole('button', { name: 'Libérer' }).click();
		expect(connection.lastSent()).toEqual({ type: 'releaseHeadset', address: ADDRESS });
		connection.receive({
			type: 'headsetState',
			address: ADDRESS,
			state: connectedState({ link: 'cut' })
		});
		await expect.element(page.getByText(/Connexion coupée/)).toBeVisible();
		await page.getByRole('button', { name: 'Reprendre la connexion' }).click();
		expect(connection.lastSent()).toEqual({ type: 'resumeHeadset', address: ADDRESS });
	});

	it('shows what the headphones said but does not let it be changed while the link is closed', async () => {
		await renderPage(connectedState({ link: 'released' }));
		await expect.element(page.getByRole('radio', { name: 'Désactivé' }).first()).toBeDisabled();
		await expect.element(page.getByRole('slider', { name: 'Égaliseur : 1 kHz' })).toBeDisabled();
	});
});
