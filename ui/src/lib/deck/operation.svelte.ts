import type { Problem } from './problems.js';

/**
 * An action the user started and waits for (upload, import): whether it runs, then what
 * came of it, as a status to announce or a problem to show.
 */
export class Operation {
	busy = $state(false);
	status = $state('');
	problem: Problem | undefined = $state();

	/**
	 * Runs `action`, which resolves the problem that stopped it, if any; on success `done`
	 * becomes the status. Ignored while another run is under way. Resolves whether it
	 * succeeded.
	 */
	async run(action: () => Promise<Problem | undefined>, done: string): Promise<boolean> {
		if (this.busy) return false;
		this.busy = true;
		this.clear();
		const problem = await action();
		this.busy = false;
		this.problem = problem;
		if (!problem) this.status = done;
		return !problem;
	}

	/** Forgets the outcome of the last run. */
	clear(): void {
		this.status = '';
		this.problem = undefined;
	}
}
