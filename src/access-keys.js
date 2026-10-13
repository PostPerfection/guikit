// letters for buttons both apps have
export const SHARED_ACCESS_KEYS = {
  'add-video': 'v',
  'add-audio': 'a',
  'add-subtitle': 's',
  'add-cpl': 'c',
  'auto-crop': 'o',
  'select-package': 's',
  validate: 'v',
  cancel: 'c',
  'show-in-folder': 'h',
  measure: 'm',
  transcode: 't',
  'burn-in': 'b',
  'refresh-jobs': 'r',
  'show-gpu-license': 'h',
  'save-settings': 's',
  'reset-settings': 'r',
};

const ACCESS_KEY_SCOPE_SELECTOR = '.toolbar, .view.active';
const BROWSE_FIELD_SELECTOR = '.input-with-btn';
export const BROWSE_BUTTON_TEXT = '…';

const KEY_CODE_PREFIX = 'Key';
const ALT_TITLE_MARK = 'Alt+';
const SHOWING_ACCESS_KEYS_CLASS = 'show-access-keys';
const ACCESS_KEY_CLASS = 'access-key';
const MENU_SELECTOR = '[role="menu"]';
const OPEN_MENU_SELECTOR = `${MENU_SELECTOR}:not([hidden])`;
const MENU_ITEM_SELECTOR = '[role="menuitem"]';
const OPEN_NATIVE_DIALOG_SELECTOR = 'dialog[open]';
const OPEN_DIALOG_SELECTOR = '[role="dialog"]:not([hidden])';
const DIALOG_CANCEL_SELECTOR = '[data-dialog-cancel]';
const DIALOG_DEFAULT_SELECTOR = '[data-dialog-default]';
const LETTERED_SELECTOR = '[data-key], [data-shared-action]';
const WORD_START = /[\s+\-/(]/;

export function accessKeyOf({ key, action }) {
  if (key) return key.toLowerCase();
  return action ? SHARED_ACCESS_KEYS[action] ?? null : null;
}

// a letter that starts a word wins over an earlier one inside a word
export function accessKeyIndex(text, key) {
  const lower = text.toLowerCase();
  let first = -1;
  for (let index = lower.indexOf(key); index !== -1; index = lower.indexOf(key, index + 1)) {
    if (index === 0 || WORD_START.test(lower[index - 1])) return index;
    if (first === -1) first = index;
  }
  return first;
}

function buttonAccessKey(button) {
  return accessKeyOf({ key: button.dataset.key, action: button.dataset.sharedAction });
}

function isShown(element) {
  return element.getClientRects().length > 0;
}

function underlineAccessKey(button, key) {
  if (button.querySelector(`.${ACCESS_KEY_CLASS}`)) return;
  const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const index = accessKeyIndex(node.data, key);
    if (index === -1) continue;
    const letter = node.splitText(index);
    letter.splitText(1);
    const underline = document.createElement('span');
    underline.className = ACCESS_KEY_CLASS;
    letter.replaceWith(underline);
    underline.append(letter);
    return;
  }
}

// menu items take plain letters
export function titleWithAccessKey(button, title) {
  const key = buttonAccessKey(button);
  return key && !button.matches(MENU_ITEM_SELECTOR) ? `${title} (${ALT_TITLE_MARK}${key.toUpperCase()})` : title;
}

function addAccessKeyToTitle(button) {
  if (button.title.includes(ALT_TITLE_MARK)) return;
  button.title = titleWithAccessKey(button, button.title || button.textContent.trim());
}

// labels change at runtime
function decorateAccessKeys() {
  for (const button of document.querySelectorAll(LETTERED_SELECTOR)) {
    const key = buttonAccessKey(button);
    if (!key) continue;
    underlineAccessKey(button, key);
    addAccessKeyToTitle(button);
  }
}

function showAccessKeys(shown) {
  if (shown) decorateAccessKeys();
  document.body.classList.toggle(SHOWING_ACCESS_KEYS_CLASS, shown);
}

export function initAccessKeys() {
  decorateAccessKeys();
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Alt') showAccessKeys(true);
  });
  document.addEventListener('keyup', (event) => {
    if (event.key === 'Alt') showAccessKeys(false);
  });
  window.addEventListener('blur', () => showAccessKeys(false));
}

function letterOf(event) {
  return event.code.startsWith(KEY_CODE_PREFIX) ? event.code.slice(KEY_CODE_PREFIX.length).toLowerCase() : null;
}

function clickLetteredButton(scopes, letter) {
  for (const scope of scopes) {
    for (const button of scope.querySelectorAll(LETTERED_SELECTOR)) {
      if (buttonAccessKey(button) !== letter) continue;
      // the toolbar holds the Project menu, whose letters only count while it is open
      if (!scope.matches(MENU_SELECTOR) && button.closest(MENU_SELECTOR)) continue;
      if (!button.disabled && isShown(button)) button.click();
      return true;
    }
  }
  return false;
}

export function pressAccessKey(event) {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  const letter = letterOf(event);
  if (!letter) return false;
  event.preventDefault();
  return clickLetteredButton(document.querySelectorAll(ACCESS_KEY_SCOPE_SELECTOR), letter);
}

export function openBrowseDialog(field) {
  const row = field.closest(BROWSE_FIELD_SELECTOR);
  const browse = row && [...row.querySelectorAll('button')].find((button) => button.textContent.trim() === BROWSE_BUTTON_TEXT);
  if (!browse || browse.disabled) return false;
  browse.click();
  return true;
}

function menuTrigger(menu) {
  return document.querySelector(`[aria-controls="${menu.id}"]`);
}

function moveMenuFocus(menu, step) {
  const items = [...menu.querySelectorAll(MENU_ITEM_SELECTOR)].filter((item) => !item.disabled && isShown(item));
  if (!items.length) return;
  const current = items.indexOf(document.activeElement.closest(MENU_ITEM_SELECTOR));
  const next = current === -1 ? (step > 0 ? 0 : items.length - 1) : (current + step + items.length) % items.length;
  items[next].focus();
}

function handleMenuKey(event, menu) {
  if (event.key === 'Escape') {
    const trigger = menuTrigger(menu);
    trigger?.click();
    trigger?.focus();
  } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    moveMenuFocus(menu, event.key === 'ArrowDown' ? 1 : -1);
  } else if (event.key === 'Enter' && event.target.matches(MENU_ITEM_SELECTOR) && event.target.tagName !== 'BUTTON') {
    event.target.click();
  } else if (!event.ctrlKey && !event.metaKey && letterOf(event)) {
    clickLetteredButton([menu], letterOf(event));
  } else {
    return;
  }
  event.preventDefault();
}

function handleDialogKey(event, dialog) {
  let button = null;
  if (event.key === 'Escape') button = dialog.querySelector(DIALOG_CANCEL_SELECTOR);
  else if (event.key === 'Enter' && !['BUTTON', 'TEXTAREA'].includes(event.target.tagName)) button = dialog.querySelector(DIALOG_DEFAULT_SELECTOR);
  if (!button) return;
  event.preventDefault();
  if (!button.disabled) button.click();
}

// true while an open menu or dialog takes the keyboard
export function handleKeyboardScopeKey(event) {
  // the browser handles Escape on a native dialog
  if (document.querySelector(OPEN_NATIVE_DIALOG_SELECTOR)) return true;
  const dialog = document.querySelector(OPEN_DIALOG_SELECTOR);
  if (dialog) {
    handleDialogKey(event, dialog);
    return true;
  }
  const menu = document.querySelector(OPEN_MENU_SELECTOR);
  if (!menu) return false;
  handleMenuKey(event, menu);
  return true;
}

export function focusFirstMenuItem(menu) {
  menu.querySelector(MENU_ITEM_SELECTOR)?.focus();
}
