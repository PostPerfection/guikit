// preview.js imports the tauri bridge from '@tauri-apps/api/core', which is not
// installed here, so the harness resolves that one specifier to the stub. Node
// runs this off the main thread, which is why the file holds nothing but the
// mapping.

const STUB = new URL('./tauri-core-stub.mjs', import.meta.url).href;

export function resolve(specifier, context, nextResolve) {
  if (specifier === '@tauri-apps/api/core') return { url: STUB, shortCircuit: true };
  return nextResolve(specifier, context);
}
