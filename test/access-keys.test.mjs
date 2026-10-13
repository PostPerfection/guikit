import assert from 'node:assert/strict';
import test from 'node:test';

import { accessKeyIndex, accessKeyOf } from '../src/access-keys.js';
import { effectiveBinding } from '../src/shortcuts.js';
import { accessKeyProblems } from './access-key-check.mjs';

test('the underline goes on a letter that starts a word before one inside a word', () => {
  assert.equal(accessKeyIndex('Make KDM', 'k'), 5);
  assert.equal(accessKeyIndex('+ Video', 'v'), 2);
  assert.equal(accessKeyIndex('Convert', 'v'), 3);
  assert.equal(accessKeyIndex('Show in folder', 'h'), 1);
  assert.equal(accessKeyIndex('Export', 'q'), -1);
});

test('a written letter is used as is, a shared action takes the letter both apps use', () => {
  assert.equal(accessKeyOf({ key: 'X' }), 'x');
  assert.equal(accessKeyOf({ action: 'validate' }), 'v');
  assert.equal(accessKeyOf({}), null);
});

const TOOLBAR = `
  <header class="toolbar">
    <button id="btn-project-menu" data-key="p">Project</button>
    <div id="project-menu" role="menu" hidden>
      <button id="btn-new-project" data-key="n" role="menuitem">New</button>
      <button id="btn-project-open" data-key="o" role="menuitem">Open</button>
    </div>
    <button id="btn-build">Build</button>
  </header>`;

function page(view) {
  return `${TOOLBAR}<div class="view" id="view-tools">${view}</div>`;
}

test('a page whose view buttons all have distinct letters from their labels has no problems', () => {
  const view = `
    <section data-submit="run-encode">
      <div class="input-with-btn"><input id="source"><button id="browse-source">…</button></div>
      <button id="run-encode" data-key="e">Encode</button>
      <button id="run-open" data-key="o">Open</button>
      <button id="run-validate" data-shared-action="validate">Validate</button>
      <button id="zoom-in" data-no-key>+</button>
    </section>`;
  assert.deepEqual(accessKeyProblems(page(view)), []);
});

test('the checker names a clash, a missing letter, a letter not in the label and an unknown action', () => {
  const view = `
    <button id="first" data-key="e">Encode</button>
    <button id="second" data-key="e">Export</button>
    <button id="bare">Measure</button>
    <button id="wrong" data-key="z">Convert</button>
    <button id="unknown" data-shared-action="frobnicate">Frobnicate</button>`;
  assert.deepEqual(accessKeyProblems(page(view)), [
    'wrong takes z, which is not in its label',
    'unknown names the unknown action frobnicate',
    'view-tools gives e to first and second',
    'bare in view-tools has no letter',
  ]);
});

test('the Project button letter is taken in every view', () => {
  assert.deepEqual(accessKeyProblems(page('<button id="preview" data-key="p">Preview</button>')), [
    'view-tools gives p to btn-project-menu and preview',
  ]);
});

test('a section that names a missing button is reported', () => {
  assert.deepEqual(accessKeyProblems(page('<section data-submit="gone"></section>')), [
    'data-submit names gone, which is not a button',
  ]);
});

test('a binding you chose wins over a default that clashes with it', () => {
  const versionFile = { id: 'view-vf', binding: 'Ctrl+6' };
  const jobs = { id: 'view-jobs', binding: 'Ctrl+8' };
  const overrides = { 'some-action': 'Ctrl+6' };
  assert.equal(effectiveBinding(versionFile, overrides), null);
  assert.equal(effectiveBinding(jobs, overrides), 'Ctrl+8');
  assert.equal(effectiveBinding({ id: 'some-action', binding: 'Ctrl+1' }, overrides), 'Ctrl+6');
});
