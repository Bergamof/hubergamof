import { execFileSync } from 'node:child_process';
import { REPOSITORY_ROOT } from './demo-manager.js';

/**
 * Builds the manager once: every demo test starts its own instance of it, which serves
 * `ui/build/` (made by the `webServer` of the Playwright config, read at run time by debug builds).
 */
export default function globalSetup(): void {
	execFileSync('cargo', ['build', '--quiet', '--locked', '--package', 'hubergamof'], {
		cwd: REPOSITORY_ROOT,
		stdio: 'inherit'
	});
}
