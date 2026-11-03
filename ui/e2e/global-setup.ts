import { execFileSync } from 'node:child_process';
import { REPOSITORY_ROOT } from './demo-manager.js';

/**
 * Builds the UI then the manager once: every demo test starts its own instance of it, which
 * serves `ui/build/` (read at run time by debug builds).
 */
export default function globalSetup(): void {
	execFileSync('pnpm', ['build'], { stdio: 'inherit' });
	execFileSync('cargo', ['build', '--quiet', '--locked', '--package', 'bergadeck'], {
		cwd: REPOSITORY_ROOT,
		stdio: 'inherit'
	});
}
