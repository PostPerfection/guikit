import { accessKeyIndex, accessKeyOf, BROWSE_BUTTON_TEXT, SHARED_ACCESS_KEYS } from '../src/access-keys.js';

const TAG = /<(\/?)([a-zA-Z][\w-]*)([^>]*?)(\/?)>|([^<]+)/g;
const ATTRIBUTE = /([\w:-]+)(?:\s*=\s*"([^"]*)")?/g;
const VOID_TAGS = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const LETTER = /^[a-z]$/;
const ENTITIES = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'" };

function attributesOf(text) {
  const attributes = {};
  for (const [, name, value = ''] of text.matchAll(ATTRIBUTE)) attributes[name] = value;
  return attributes;
}

function decoded(text) {
  return text.replace(/&(amp|lt|gt|quot|#39);/g, (entity) => ENTITIES[entity]);
}

function scopeOf(tag, attributes) {
  const classes = (attributes.class || '').split(/\s+/);
  if (tag === 'header' && classes.includes('toolbar')) return 'toolbar';
  if (attributes.role === 'menu') return `menu ${attributes.id}`;
  if (classes.includes('view') && attributes.id?.startsWith('view-')) return attributes.id;
  return null;
}

// every button with the scope it sits in: the toolbar, a view, a menu, or none
export function pageButtons(page) {
  const open = [];
  const buttons = [];
  let button = null;
  for (const [, closing, rawTag, attributeText, selfClosing, text] of page.matchAll(TAG)) {
    if (text !== undefined) {
      if (button) button.text += text;
      continue;
    }
    const tag = rawTag.toLowerCase();
    if (closing) {
      const index = open.findLastIndex((element) => element.tag === tag);
      if (index !== -1) open.length = index;
      if (tag === 'button' && button) {
        button.text = decoded(button.text).trim();
        buttons.push(button);
        button = null;
      }
      continue;
    }
    const attributes = attributesOf(attributeText);
    if (tag === 'button') {
      const scope = open.findLast((element) => element.scope)?.scope ?? null;
      button = { attributes, text: '', scope };
    }
    if (VOID_TAGS.has(tag) || selfClosing) continue;
    open.push({ tag, scope: scopeOf(tag, attributes) });
  }
  return buttons;
}

function describe(button) {
  return button.attributes.id || `"${button.text}"`;
}

function letterProblems(button) {
  const { 'data-key': key, 'data-shared-action': action } = button.attributes;
  if (key !== undefined && action !== undefined) return [`${describe(button)} has both data-key and data-shared-action`];
  if (action !== undefined && !(action in SHARED_ACCESS_KEYS)) return [`${describe(button)} names the unknown action ${action}`];
  if (key !== undefined && !LETTER.test(key)) return [`${describe(button)} has data-key "${key}", not one lowercase letter`];
  const letter = accessKeyOf({ key, action });
  if (letter && accessKeyIndex(button.text, letter) === -1) return [`${describe(button)} takes ${letter}, which is not in its label`];
  return [];
}

function clashes(buttons, scopeName) {
  const byLetter = new Map();
  for (const button of buttons) {
    const letter = accessKeyOf({ key: button.attributes['data-key'], action: button.attributes['data-shared-action'] });
    if (!letter) continue;
    if (!byLetter.has(letter)) byLetter.set(letter, []);
    byLetter.get(letter).push(describe(button));
  }
  return [...byLetter]
    .filter(([, names]) => names.length > 1)
    .map(([letter, names]) => `${scopeName} gives ${letter} to ${names.join(' and ')}`);
}

function isUnlettered(button) {
  const { 'data-key': key, 'data-shared-action': action, 'data-no-key': exempt } = button.attributes;
  return key === undefined && action === undefined && exempt === undefined && button.text !== BROWSE_BUTTON_TEXT;
}

export function accessKeyProblems(page) {
  const buttons = pageButtons(page);
  const problems = buttons.flatMap(letterProblems);
  const toolbar = buttons.filter((button) => button.scope === 'toolbar');
  const scopes = new Set(buttons.map((button) => button.scope).filter((scope) => scope && scope !== 'toolbar'));
  for (const scope of scopes) {
    const inScope = buttons.filter((button) => button.scope === scope);
    const isView = scope.startsWith('view-');
    problems.push(...clashes(isView ? [...toolbar, ...inScope] : inScope, scope));
    if (isView) problems.push(...inScope.filter(isUnlettered).map((button) => `${describe(button)} in ${scope} has no letter`));
  }
  const ids = new Set(buttons.map((button) => button.attributes.id).filter(Boolean));
  for (const [, target] of page.matchAll(/data-submit="([^"]*)"/g)) {
    if (!ids.has(target)) problems.push(`data-submit names ${target}, which is not a button`);
  }
  return problems;
}
