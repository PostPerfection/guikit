// The modules import the tauri bridge from '@tauri-apps/api/core' and the window
// from '@tauri-apps/api/window', neither installed here, so the harness resolves
// those two specifiers to stubs. Node runs this off the main thread, which is why
// the file holds nothing but the mapping.

const STUBS = new Map([
  ['@tauri-apps/api/core', new URL('./tauri-core-stub.mjs', import.meta.url).href],
  ['@tauri-apps/api/window', new URL('./tauri-window-stub.mjs', import.meta.url).href],
]);

export function resolve(specifier, context, nextResolve) {
  if (STUBS.has(specifier)) return { url: STUBS.get(specifier), shortCircuit: true };
  return nextResolve(specifier, context);
}
