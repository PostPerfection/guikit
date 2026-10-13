// the request each finished build was submitted with, by the package folder it wrote
function storedBuilds(storageKey) {
  return JSON.parse(localStorage.getItem(storageKey) || '{}');
}

export function rememberFinishedBuild(storageKey, packagePath, requestText) {
  localStorage.setItem(storageKey, JSON.stringify({ ...storedBuilds(storageKey), [packagePath]: requestText }));
}

export function finishedBuildRequest(storageKey, packagePath) {
  return storedBuilds(storageKey)[packagePath] ?? null;
}

export function forgetFinishedBuild(storageKey, packagePath) {
  const { [packagePath]: _forgotten, ...kept } = storedBuilds(storageKey);
  localStorage.setItem(storageKey, JSON.stringify(kept));
}
