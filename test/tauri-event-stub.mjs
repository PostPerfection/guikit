// The tauri event bridge, standing in for @tauri-apps/api/event, so a test can
// emit what the backend would.

const listeners = new Map();

export async function listen(name, handler) {
  if (!listeners.has(name)) listeners.set(name, new Set());
  listeners.get(name).add(handler);
  return () => listeners.get(name).delete(handler);
}

export function emit(name, payload) {
  for (const handler of listeners.get(name) ?? []) handler({ payload });
}

export function listenerCount(name) {
  return listeners.get(name)?.size ?? 0;
}
