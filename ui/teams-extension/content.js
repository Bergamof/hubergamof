// A content script cannot be an ES module: it loads the module that does the work.
(async () => {
	const { start } = await import(chrome.runtime.getURL('lib/content-main.js'));
	start(document, chrome.runtime);
})();
