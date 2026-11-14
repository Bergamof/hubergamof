<!--
@component
A field for a whole number within bounds, with its unit, a hint and an error shown while what is typed
is not accepted. The value is sent as soon as it is valid; leaving the field shows the saved value again.
-->
<script lang="ts">
	interface Props {
		label: string;
		/** Saved value. */
		value: number;
		min: number;
		max: number;
		/** Unit shown after the field, read with its label. */
		unit: string;
		/** What the value does, shown under the field. */
		hint: string;
		/** Said when what is typed is not accepted. */
		error: string;
		onchange: (value: number) => void;
	}

	let { label, value, min, max, unit, hint, error, onchange }: Props = $props();

	const uid = $props.id();

	let draft: string | undefined = $state();
	const text = $derived(draft ?? String(value));
	const valid = $derived(parse(text) !== undefined);

	/** The whole number written in `typed`, if within bounds. */
	function parse(typed: string): number | undefined {
		const parsed = Number(typed);
		return typed.trim() !== '' && Number.isInteger(parsed) && parsed >= min && parsed <= max
			? parsed
			: undefined;
	}

	function input(typed: string) {
		draft = typed;
		const parsed = parse(typed);
		if (parsed !== undefined) {
			onchange(parsed);
		}
	}
</script>

<div class="flex flex-col gap-1">
	<label class="font-medium" for="{uid}-field">{label}</label>
	<span class="flex items-center gap-2">
		<input
			id="{uid}-field"
			type="text"
			inputmode="numeric"
			autocomplete="off"
			class="field w-24"
			value={text}
			oninput={(event) => input(event.currentTarget.value)}
			onblur={() => (draft = undefined)}
			aria-describedby="{uid}-hint{valid ? '' : ` ${uid}-error`}"
			aria-invalid={valid ? undefined : true}
		/>
		<span class="text-sm text-muted">{unit}</span>
	</span>
	<p id="{uid}-hint" class="text-sm text-muted">{hint}</p>
	{#if !valid}
		<p id="{uid}-error" role="alert" class="text-sm font-medium text-danger">{error}</p>
	{/if}
</div>
