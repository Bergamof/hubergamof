<!--
@component
A keyboard shortcut: typed (`ctrl+shift+m`) or recorded by pressing the keys. The value is sent when the
field is left or Enter is pressed, never while it is half typed. While recording, Tab leaves as usual
(it cannot be part of a shortcut recorded here), so the keyboard is never trapped.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import {
		learnKey,
		loadKeyboardLayout,
		mergeLayouts,
		readLearnedLayout,
		saveLearnedLayout,
		type KeyboardLayout,
		type KeyPress
	} from '#lib/deck/keyboard-layout.js';
	import {
		isReserved,
		layoutKnowsShortcut,
		keyName,
		modifierName,
		normalizeShortcut,
		shortcutLabel,
		shortcutOf,
		shortcutOnLayout
	} from '#lib/deck/shortcut.js';

	interface Props {
		label: string;
		/** Saved shortcut, `ctrl+shift+m`. */
		value: string;
		onchange: (shortcut: string) => void;
		/** Puts the focus in the field when shown (it appears as the next step of a choice). */
		focusOnMount?: boolean;
		/** What the keys produce on the keyboard of the person; read from the browser when not given. */
		layout?: KeyboardLayout;
	}

	let { label, value, onchange, focusOnMount = false, layout }: Props = $props();

	const uid = $props.id();

	let field: HTMLInputElement | undefined = $state();
	/** Said by the status region: what was saved, read again even if identical. */
	let announcement = $state('');
	let draft: string | undefined = $state();
	let recording = $state(false);
	/** Why the last attempt was not accepted. */
	let refusal: string | undefined = $state();
	/** Keys pressed since the recording started. */
	let held: string[] = [];
	let pressedOther = false;

	const text = $derived(draft ?? value);

	let loadedLayout: KeyboardLayout | undefined = $state();
	let learned: KeyboardLayout = $state(new Map());
	/** What the keys write: what the browser tells, and what the keys pressed here have taught. */
	const known = $derived(layout ?? mergeLayouts(learned, loadedLayout));
	const onKeyboard = $derived(value ? shortcutOnLayout(value, known) : undefined);
	const unknownKeyboard = $derived(Boolean(value) && !layoutKnowsShortcut(value, known));

	$effect(() => {
		if (layout !== undefined) return;
		learned = readLearnedLayout();
		void loadKeyboardLayout().then((loaded) => (loadedLayout = loaded));
	});

	$effect(() => {
		if (focusOnMount) field?.focus();
	});

	/** A key pressed anywhere on the page tells what it writes (not when the layout is given). */
	function learn(press: KeyPress) {
		if (layout !== undefined) return;
		const next = learnKey(learned, press);
		if (!next) return;
		learned = next;
		saveLearnedLayout(next);
	}

	async function announce(message: string) {
		announcement = '';
		await tick();
		announcement = message;
	}

	/** Sends `shortcut` when it can be pressed; otherwise says why. */
	function commit(shortcut: string | undefined) {
		if (shortcut === undefined) {
			refusal = 'Une de ces touches ne peut pas être envoyée.';
		} else if (isReserved(shortcut)) {
			refusal =
				'Cette combinaison (Ctrl + Alt avec Suppr, Retour arrière ou une touche F) est réservée.';
		} else {
			refusal = undefined;
			draft = undefined;
			if (shortcut !== value) {
				onchange(shortcut);
				void announce(`Raccourci ${shortcutLabel(shortcut)}.`);
			}
		}
	}

	function typed(event: Event & { currentTarget: HTMLInputElement }) {
		const shortcut = normalizeShortcut(event.currentTarget.value);
		if (shortcut === undefined) {
			draft = event.currentTarget.value;
			refusal = 'Touches inconnues : écrivez par exemple ctrl+shift+m.';
		} else {
			commit(shortcut);
		}
	}

	function startRecording() {
		held = [];
		pressedOther = false;
		refusal = undefined;
		recording = true;
	}

	function stopRecording() {
		recording = false;
	}

	function keydown(event: KeyboardEvent) {
		if (!recording || event.code === 'Tab') return;
		event.preventDefault();
		if (event.repeat) return;
		if (!held.includes(event.code)) held.push(event.code);
		if (modifierName(event.code) === undefined) {
			pressedOther = true;
			commit(shortcutOf(held));
			stopRecording();
		}
	}

	function keyup(event: KeyboardEvent) {
		// No key pressed since the start: the release of the key that pressed the button (Enter).
		if (!recording || pressedOther || held.length === 0 || keyName(event.code) === undefined)
			return;
		event.preventDefault();
		// Only modifiers were pressed: the shortcut is made of them.
		commit(shortcutOf(held));
		stopRecording();
	}
</script>

<svelte:window onkeydown={learn} />

<div class="flex flex-col gap-1">
	<label class="font-medium" for="{uid}-field">{label}</label>
	<span class="flex flex-wrap items-center gap-2">
		<input
			id="{uid}-field"
			type="text"
			autocomplete="off"
			spellcheck="false"
			class="field w-56"
			bind:this={field}
			value={text}
			onchange={typed}
			aria-describedby="{uid}-hint{refusal ? ` ${uid}-error` : ''}"
			aria-invalid={refusal ? true : undefined}
		/>
		<button
			type="button"
			class="secondary-button"
			aria-pressed={recording}
			onclick={() => (recording ? stopRecording() : startRecording())}
			onkeydown={keydown}
			onkeyup={keyup}
			onblur={stopRecording}
		>
			Enregistrer en appuyant sur les touches
		</button>
	</span>
	<p id="{uid}-hint" class="text-sm text-muted">
		{value ? `Envoie ${shortcutLabel(value)}.` : 'Exemple : ctrl+shift+m.'}
		{#if onKeyboard}
			Sur votre clavier, ces touches s’écrivent {onKeyboard}.
		{/if}
		Touches de la position américaine du clavier. Le champ est validé en le quittant ou avec Entrée ;
		pendant l’enregistrement, appuyez sur les touches voulues, Tab pour annuler.
		{#if unknownKeyboard}
			Pour voir ces touches telles qu’elles sont écrites sur votre clavier, appuyez une fois dessus.
		{/if}
	</p>
	{#if recording}
		<p class="text-sm font-medium">Enregistrement : appuyez sur la combinaison…</p>
	{/if}
	<p class="text-sm text-muted">
		Le navigateur garde certaines combinaisons (Ctrl + Q, Ctrl + W, Ctrl + T, Ctrl + N…) : elles ne
		peuvent pas être enregistrées en appuyant sur les touches. Écrivez-les dans le champ, par
		exemple ctrl+q.
	</p>
	<p role="status" class="sr-only">{announcement}</p>
	{#if refusal}
		<p id="{uid}-error" role="alert" class="text-sm font-medium text-danger">{refusal}</p>
	{/if}
</div>
