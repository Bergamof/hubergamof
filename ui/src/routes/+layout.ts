// The manager serves the UI as static files, and the page is the same shell for every address: the state of
// the manager is only known once the browser has connected to it, so nothing is rendered at build time.
export const prerender = true;
export const ssr = false;
