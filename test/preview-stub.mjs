// The preview player as the playlist sees it, standing in for preview.js so the
// queue can be driven with no webview, no tauri and no mpv. Everything the
// playlist asks for is recorded in order, and `poll` and `loadedByTheApp` are the
// two things the real player would call back with.

// What the queue asked the player to do, in order: ['load', path],
// ['play_pause'], ['stop'] or ['close'].
export const calls = [];

const watchers = {
  metadata: () => {},
  load: () => {},
};

export function previewDcp(dirPath) {
  calls.push(['load', dirPath]);
  // the real one reports every load it is asked for before it makes it
  watchers.load(dirPath);
}

export function previewPlayPause() {
  calls.push(['play_pause']);
}

export function stopPreview() {
  calls.push(['stop']);
}

export function closePreview() {
  calls.push(['close']);
}

export function watchPreviewMetadata(watcher) {
  watchers.metadata = watcher;
}

export function watchPreviewLoads(watcher) {
  watchers.load = watcher;
}

/// One metadata poll, the shape preview.js parses out of `preview_get_metadata`.
export function poll(meta) {
  watchers.metadata({ position: 0, duration: 10, paused: false, filename: null, eof: false, ...meta });
}

/// A load the app made itself, which is what takes the screen off the queue.
export function loadedByTheApp(path) {
  watchers.load(path);
}

export function forgetCalls() {
  calls.length = 0;
}
