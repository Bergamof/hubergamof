// Bergadeck: activates the window ID (constant above), restored if minimised.
const target = workspace.windowList().find((window) => String(window.internalId) === ID);
if (target) {
    target.minimized = false;
    workspace.activeWindow = target;
}
