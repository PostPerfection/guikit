const STUB = new URL('./tauri-plugins-stub.mjs', import.meta.url).href;
const STUBBED = new Set([
  '@tauri-apps/plugin-dialog',
  '@tauri-apps/plugin-fs',
  '@tauri-apps/api/path',
  '@tauri-apps/api/window',
  '@tauri-apps/api/core',
  '@tauri-apps/api/event',
]);

export function resolve(specifier, context, nextResolve) {
  if (STUBBED.has(specifier)) return { url: STUB, shortCircuit: true };
  return nextResolve(specifier, context);
}
