import assert from 'node:assert/strict';
import test from 'node:test';

const storage = new Map();
globalThis.localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => storage.set(key, String(value)),
};

const { rememberFinishedBuild, finishedBuildRequest, forgetFinishedBuild } = await import('../src/finished-builds.js');

const KEY = 'dcpwizard-finished-builds';

test('a finished build is found again by its package folder until it is forgotten', () => {
  rememberFinishedBuild(KEY, '/out/Film', '{"title":"Film"}');
  rememberFinishedBuild(KEY, '/out/Short', '{"title":"Short"}');
  assert.equal(finishedBuildRequest(KEY, '/out/Film'), '{"title":"Film"}');

  forgetFinishedBuild(KEY, '/out/Film');
  assert.equal(finishedBuildRequest(KEY, '/out/Film'), null);
  assert.equal(finishedBuildRequest(KEY, '/out/Short'), '{"title":"Short"}');
});

test('a package built again keeps only its latest request', () => {
  rememberFinishedBuild(KEY, '/out/Film', 'first');
  rememberFinishedBuild(KEY, '/out/Film', 'second');
  assert.equal(finishedBuildRequest(KEY, '/out/Film'), 'second');
});
