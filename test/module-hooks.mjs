// playlist.js imports the preview player straight from './preview.js', so the
// harness resolves that one specifier to the stub instead. Node runs this off the
// main thread, which is why the file holds nothing but the mapping.

const STUB = new URL('./preview-stub.mjs', import.meta.url).href;

export function resolve(specifier, context, nextResolve) {
  if (specifier.endsWith('preview.js')) return { url: STUB, shortCircuit: true };
  return nextResolve(specifier, context);
}
