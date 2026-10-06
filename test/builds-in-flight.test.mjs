import assert from 'node:assert/strict';
import test from 'node:test';

let instances = 0;

// the module keeps its titles in a module-level set
async function freshBuilds() {
  return import(`../src/builds-in-flight.js?instance=${++instances}`);
}

test('a begun title is in flight until it ends', async () => {
  const builds = await freshBuilds();
  builds.beginBuild('Feature');
  assert.equal(builds.buildInFlight('Feature'), true);
  builds.endBuild('Feature');
  assert.equal(builds.buildInFlight('Feature'), false);
});

test('two titles are tracked separately', async () => {
  const builds = await freshBuilds();
  builds.beginBuild('Feature');
  builds.beginBuild('Trailer');
  builds.endBuild('Feature');
  assert.equal(builds.buildInFlight('Feature'), false);
  assert.equal(builds.buildInFlight('Trailer'), true);
});

test('ending a title that never began changes nothing', async () => {
  const builds = await freshBuilds();
  builds.beginBuild('Feature');
  builds.endBuild('Trailer');
  assert.equal(builds.buildInFlight('Feature'), true);
  assert.equal(builds.buildInFlight('Trailer'), false);
  assert.equal(builds.anyBuildInFlight(), true);
});

test('any build in flight follows the set', async () => {
  const builds = await freshBuilds();
  assert.equal(builds.anyBuildInFlight(), false);
  builds.beginBuild('Feature');
  builds.beginBuild('Trailer');
  builds.endBuild('Feature');
  assert.equal(builds.anyBuildInFlight(), true);
  builds.endBuild('Trailer');
  assert.equal(builds.anyBuildInFlight(), false);
});
