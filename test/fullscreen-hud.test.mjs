import assert from 'node:assert/strict';
import test from 'node:test';

const { fullscreenHudStaysShown, pointIsInsideAnyRectangle } = await import('../src/fullscreen-hud.js');

const HEADER = { left: 0, top: 0, right: 1920, bottom: 32 };
const TRANSPORT = { left: 0, top: 1040, right: 1920, bottom: 1080 };

test('the controls hide only while playing with the pointer off them', () => {
  assert.equal(fullscreenHudStaysShown({ paused: false, pointerOverHud: false }), false);
  assert.equal(fullscreenHudStaysShown({ paused: true, pointerOverHud: false }), true);
  assert.equal(fullscreenHudStaysShown({ paused: false, pointerOverHud: true }), true);
});

test('a point counts as inside any rectangle that holds it, edges included', () => {
  assert.equal(pointIsInsideAnyRectangle({ x: 960, y: 1060 }, [HEADER, TRANSPORT]), true);
  assert.equal(pointIsInsideAnyRectangle({ x: 1920, y: 32 }, [HEADER, TRANSPORT]), true);
  assert.equal(pointIsInsideAnyRectangle({ x: 960, y: 540 }, [HEADER, TRANSPORT]), false);
  assert.equal(pointIsInsideAnyRectangle({ x: 960, y: 1060 }, []), false);
});
