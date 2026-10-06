// The drag label in src/drag-label.js: the real module, a fake document and a
// fake dragstart event that records what it was given. `node --test test/`.
import assert from 'node:assert/strict';
import test from 'node:test';

import { setDragLabel, shortenLabel } from '../src/drag-label.js';

const LONG_NAME = 'The Toms_Rec709-2.4_DNxHR HQX_24-bit-48kHz-LinearPCM_4096x1716.mov';

test('a short name is left alone', () => {
  assert.equal(shortenLabel('movie.mov'), 'movie.mov');
});

test('a long name is cut in the middle to the limit and keeps its extension', () => {
  const shortened = shortenLabel(LONG_NAME);
  assert.equal(shortened.length, 40);
  assert.ok(shortened.startsWith('The Toms_Rec709'));
  assert.ok(shortened.endsWith('4096x1716.mov'));
  assert.ok(shortened.includes('…'));
});

test('the drag image is a label element carrying the shortened name', async () => {
  const appended = [];
  const removed = [];
  globalThis.document = {
    createElement: (tag) => ({ tag, className: '', textContent: '', remove() { removed.push(this); } }),
    body: { appendChild: (element) => appended.push(element) },
  };
  const images = [];
  const event = { dataTransfer: { setDragImage: (...args) => images.push(args) } };

  setDragLabel(event, LONG_NAME);

  assert.equal(appended.length, 1);
  assert.equal(appended[0].className, 'drag-label');
  assert.equal(appended[0].textContent, shortenLabel(LONG_NAME));
  assert.deepEqual(images, [[appended[0], 0, 0]]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(removed, [appended[0]]);
  delete globalThis.document;
});
