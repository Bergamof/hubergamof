<!--
@component
A keyboard shortcut: typed (`ctrl+shift+m`) or recorded by pressing the keys. The value is sent when the
field is left or Enter is pressed, never while it is half typed. While recording, Tab leaves as usual
(it cannot be part of a shortcut recorded here), so the keyboard is never trapped.
-->
<script lang="ts">
	import {
		isReserved,
		keyName,
		modifierName,
		normalizeShortcut,
		shortcutLabel,
		shortcutOf
	} from '#lib/deck/shortcut.js';

	interface Props {
		label: string;
		/** Saved shortcut, `ctrl+shift+m`. */
		value: string;
		onchange: (shortcut: string) => void;
	}

	let { label, value, onchange }: Props = $props();

	const uid = $props.id();

	let draft: string | undefined = $state();
	let recording = $state(false);
	/** Why the last attempt was not accepted. */
	let refusal: string | undefined = $state();
	/** Keys pressed since the recording started. */
	let held: string[] = [];
	let pressedOther = false;

	const text = $derived(draft ?? value);

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
			if (shortcut !== value) onchange(shortcut);
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
		if (!recording || pressedOther || keyName(event.code) === undefined) return;
		event.preventDefault();
		// Only modifiers were pressed: the shortcut is made of them.
		if (held.length > 0) commit(shortcutOf(held));
		stopRecording();
	}
</script>

<div class="flex flex-col gap-1">
	<label class="font-medium" for="{uid}-field">{label}</label>
	<span class="flex flex-wrap items-center gap-2">
		<input
			id="{uid}-field"
			type="text"
			autocomplete="off"
			spellcheck="false"
			class="field w-56"
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
			{recording ? 'Appuyez sur les touches…' : 'Enregistrer en appuyant sur les touches'}
		</button>
	</span>
	<p id="{uid}-hint" class="text-sm text-muted">
		{value ? `Envoie ${shortcutLabel(value)}.` : 'Exemple : ctrl+shift+m.'}
		Touches de la position américaine du clavier.
	</p>
	{#if refusal}
		<p id="{uid}-error" role="alert" class="text-sm font-medium text-danger">{refusal}</p>
	{/if}
</div>
