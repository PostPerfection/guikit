export const APP_DATA_FOLDER = '/data/app';

// path to text for files, path to null for folders
export const disk = new Map();
export const writes = [];
export const messages = [];

const dialogAnswers = { open: [], save: [], confirm: [] };
export const dialogRequests = { open: [], save: [], confirm: [] };
export let closeHandler = null;
const commandAnswers = new Map();
export const eventListeners = new Map();

export function resetDisk(entries = {}) {
  disk.clear();
  writes.length = 0;
  messages.length = 0;
  for (const [path, text] of Object.entries(entries)) disk.set(path, text);
}

export function answerDialog(kind, answer) {
  dialogAnswers[kind].push(answer);
}

// each answer is given once, then the command returns null
export function answerCommand(command, answer) {
  commandAnswers.set(command, answer);
}

export async function readTextFile(path) {
  if (typeof disk.get(path) !== 'string') throw new Error(`failed to open file at path: ${path}`);
  return disk.get(path);
}

export async function writeTextFile(path, text) {
  disk.set(path, text);
  writes.push(path);
}

export async function exists(path) {
  return disk.has(path);
}

export async function mkdir(path) {
  disk.set(path, null);
}

export async function remove(path) {
  if (!disk.delete(path)) throw new Error(`no such file: ${path}`);
}

export async function rename(fromPath, toPath) {
  disk.set(toPath, disk.get(fromPath));
  disk.delete(fromPath);
}

export async function open(options) {
  dialogRequests.open.push(options);
  return dialogAnswers.open.shift() ?? null;
}

export async function save(options) {
  dialogRequests.save.push(options);
  return dialogAnswers.save.shift() ?? null;
}

export async function confirm(text) {
  dialogRequests.confirm.push(text);
  return dialogAnswers.confirm.shift() ?? false;
}

export async function message(text, options) {
  messages.push([text, options]);
}

export async function appDataDir() {
  return APP_DATA_FOLDER;
}

export function getCurrentWindow() {
  return {
    async onCloseRequested(handler) {
      closeHandler = handler;
    },
  };
}

export async function invoke(command) {
  const answer = commandAnswers.get(command) ?? null;
  commandAnswers.delete(command);
  return answer;
}

export async function listen(name, handler) {
  eventListeners.set(name, handler);
  return () => eventListeners.delete(name);
}
