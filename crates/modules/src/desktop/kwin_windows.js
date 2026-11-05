// HuBergamof: reports the followed windows to the manager (constants above).
// Only the windows of the followed classes are reported.

function followed(window) {
    if (!window || !window.normalWindow) {
        return false;
    }
    const windowClass = String(window.resourceClass);
    return CLASSES.includes(windowClass)
        || PREFIXES.some((prefix) => windowClass.startsWith(prefix));
}

function describe(window) {
    return {
        id: String(window.internalId),
        class: String(window.resourceClass),
        caption: String(window.caption),
        pid: window.pid > 0 ? window.pid : 0,
        demandsAttention: Boolean(window.demandsAttention),
        active: workspace.activeWindow === window,
        minimized: Boolean(window.minimized),
    };
}

function report(removed) {
    const windows = workspace.windowList()
        .filter((window) => window !== removed && followed(window))
        .map(describe);
    callDBus(BUS, PATH, INTERFACE, "Report", JSON.stringify(windows));
}

function follow(window) {
    if (!followed(window)) {
        return;
    }
    window.captionChanged.connect(() => report());
    window.demandsAttentionChanged.connect(() => report());
    window.minimizedChanged.connect(() => report());
}

// Whether the active window is a followed one: focus changes between
// other windows are not reported.
let followedActive = followed(workspace.activeWindow);

workspace.windowList().forEach(follow);
workspace.windowAdded.connect((window) => {
    follow(window);
    if (followed(window)) {
        report();
    }
});
workspace.windowRemoved.connect((window) => {
    if (followed(window)) {
        report(window);
    }
});
workspace.windowActivated.connect((window) => {
    const nowFollowed = followed(window);
    if (nowFollowed || followedActive) {
        report();
    }
    followedActive = nowFollowed;
});
report();
