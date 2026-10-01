import { open, save, message, confirm } from '@tauri-apps/plugin-dialog';
import { readTextFile, writeTextFile, exists, mkdir, remove, rename } from '@tauri-apps/plugin-fs';
import { appDataDir } from '@tauri-apps/api/path';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { escapeHtml } from './html.js';

const FIRST_PROJECT_FILE_VERSION = 1;

export const PROJECT_FILE_SHORTCUTS = [
  { id: 'new-project', label: 'New project', binding: 'Ctrl+N', buttonId: 'btn-new-project' },
  { id: 'project-open', label: 'Open project', binding: 'Ctrl+O', buttonId: 'btn-project-open' },
  { id: 'project-save', label: 'Save project', binding: 'Ctrl+S', buttonId: 'btn-project-save' },
  { id: 'project-save-as', label: 'Save project as', binding: 'Ctrl+Shift+S', buttonId: 'btn-project-save-as' },
];

const SHORTCUT_HANDLERS = {
  'new-project': () => newProject(),
  'project-open': () => openProject(),
  'project-save': () => saveProject(),
  'project-save-as': () => saveProjectAs(),
};

const RECENT_SECTION_ID = 'recent-projects';
const RECENT_LIST_ID = 'recent-list';
const RECENT_HEADER_ID = 'recent-header';
const RECENT_TOGGLE_ID = 'recent-toggle';
const PROJECT_NAME_ID = 'project-name';
const MAX_RECENT = 20;

const DRAFT_FILE_STEM = 'draft';
const DRAFT_CHECK_MILLISECONDS = 1000;
const DRAFT_TITLE_MARK = '(draft)';
const TITLE_SEPARATOR = ' - ';
const UNTITLED_FILE_STEM = 'Untitled';
const DISCARD_CHANGES_QUESTION = 'Discard unsaved changes?';

const TAKE_LAUNCH_PROJECT_PATH_COMMAND = 'take_launch_project_path';
const OPEN_PROJECT_FILE_EVENT = 'open-project-file';

const ABSOLUTE_PATH = /^(\/|[A-Za-z]:[\\/]|\\\\)/;
const PATH_SEPARATORS = /[\\/]+/;
const FILE_NAME_UNSAFE = /[\\/:*?"<>|]/g;

let configuration = null;
let currentProjectPath = null;
let restoredFromDraft = false;
let titleStatus = '';
let draftPath = null;
let draftFolderReady = null;
// the serialized form the draft on disk, or the project file, already holds
let formOnDisk = null;
// the serialized form the open project file holds, the defaults with none open, null after a draft or an upgrade
let projectFileForm = null;
let draftOperations = Promise.resolve();
let renderGeneration = 0;
// rows keep the order they were first shown in, only the stored order tracks recency
let shownOrder = [];

export async function initProjects(options) {
  configuration = options;
  currentProjectPath = null;
  restoredFromDraft = false;
  shownOrder = [];
  for (const { id, buttonId } of PROJECT_FILE_SHORTCUTS) {
    document.getElementById(buttonId)?.addEventListener('click', SHORTCUT_HANDLERS[id]);
  }
  document.getElementById(RECENT_HEADER_ID)?.addEventListener('click', () => {
    localStorage.setItem(recentCollapsedKey(), String(!recentProjectsCollapsed()));
    applyRecentProjectsCollapsed();
  });
  updateWindowTitle();

  draftPath = joinPath(await appDataDir(), `${DRAFT_FILE_STEM}.${extension()}`);
  draftFolderReady = null;
  projectFileForm = JSON.stringify(configuration.defaults);
  await restoreDraft();
  setInterval(saveDraftIfChanged, DRAFT_CHECK_MILLISECONDS);
  await getCurrentWindow().onCloseRequested(saveDraftIfChanged);

  storeRecentProjects(await migrateRecentProjects(getRecentProjects(), extension()));
  await renderRecentProjects();

  // listen before taking the launch path, which tells the backend to start emitting this event
  await listen(OPEN_PROJECT_FILE_EVENT, (event) => openProjectFile(event.payload));
  const launchProjectPath = await invoke(TAKE_LAUNCH_PROJECT_PATH_COMMAND);
  if (launchProjectPath) await openProjectFile(launchProjectPath);
}

function extension() {
  return configuration.wizard;
}

function projectFilter() {
  return { name: `${configuration.applicationName} project`, extensions: [extension()] };
}

export function projectFileText(wizard, version, form, savedAt = new Date(), draftOf = undefined) {
  const project = { wizard, version, saved: savedAt.toISOString(), draftOf, form };
  return `${JSON.stringify(project, null, 2)}\n`;
}

export function readProjectFile(text, wizard, version, migrations) {
  let project;
  try {
    project = JSON.parse(text);
  } catch (error) {
    throw new Error(`not a project file: ${error.message}`);
  }
  if (!project || typeof project !== 'object' || typeof project.wizard !== 'string') {
    throw new Error('not a project file');
  }
  if (project.wizard !== wizard) {
    throw new Error(`this is a ${project.wizard} project, not a ${wizard} one`);
  }
  if (!Number.isInteger(project.version)) throw new Error('not a project file, it has no version');
  if (project.version < FIRST_PROJECT_FILE_VERSION) {
    throw new Error(`not a project file, version ${project.version} is below ${FIRST_PROJECT_FILE_VERSION}`);
  }
  if (project.version > version) {
    throw new Error(`saved as project version ${project.version}, this app reads up to version ${version}`);
  }
  const form = project.form && typeof project.form === 'object' && !Array.isArray(project.form) ? project.form : {};
  return { form: migratedForm(form, project.version, version, migrations), version: project.version };
}

function migratedForm(form, fileVersion, version, migrations) {
  let migrated = form;
  for (let nextVersion = fileVersion + 1; nextVersion <= version; nextVersion++) {
    const migrate = migrations[nextVersion];
    if (!migrate) throw new Error(`saved as project version ${fileVersion}, this app has no upgrade to version ${nextVersion}`);
    migrated = migrate(migrated);
  }
  return migrated;
}

function readConfiguredProjectFile(text) {
  return readProjectFile(text, configuration.wizard, configuration.projectFileVersion, configuration.projectFileMigrations);
}

// === Paths ===

function folderOf(path) {
  return path.replace(/[\\/][^\\/]*$/, '');
}

function fileNameOf(path) {
  return path.split(PATH_SEPARATORS).pop();
}

function separatorOf(path) {
  return path.includes('\\') && !path.includes('/') ? '\\' : '/';
}

function joinPath(folder, ...names) {
  return [folder.replace(/[\\/]+$/, ''), ...names].join(separatorOf(folder));
}

function fileStemOf(path) {
  return fileNameOf(path).slice(0, -(extension().length + 1));
}

function pathComponents(path) {
  return path.split(PATH_SEPARATORS).filter(Boolean);
}

export function projectPathBeside(packagePath) {
  return `${packagePath.replace(/[\\/]+$/, '')}.${extension()}`;
}

function packagePathBeside(projectPath) {
  return projectPath.slice(0, -(extension().length + 1));
}

function withExtension(path) {
  return path.endsWith(`.${extension()}`) ? path : `${path}.${extension()}`;
}

function sameComponents(first, second) {
  return first.length === second.length && first.every((component, index) => component === second[index]);
}

// longest tail first, under the project folder or overlapping its own trailing folder names
export function relocationCandidates(path, projectFolder) {
  const components = pathComponents(path);
  const folderComponents = pathComponents(projectFolder);
  const candidates = [];
  for (let length = components.length - 1; length >= 1; length--) {
    const tail = components.slice(-length);
    for (let overlap = Math.min(length, folderComponents.length); overlap >= 1; overlap--) {
      if (sameComponents(folderComponents.slice(-overlap), tail.slice(0, overlap))) {
        candidates.push(joinPath(projectFolder, ...tail.slice(overlap)));
      }
    }
    candidates.push(joinPath(projectFolder, ...tail));
  }
  return [...new Set(candidates)];
}

async function locate(path, projectFolder) {
  if (await existsInScope(path)) return path;
  if (!projectFolder) return null;
  for (const candidate of relocationCandidates(path, projectFolder)) {
    if (await existsInScope(candidate)) return candidate;
  }
  return null;
}

// output fields name files a build writes, so they need not exist yet
export async function relocateMissingPaths(form, projectFolder, outputFields = [], textFields = []) {
  const located = new Map();
  const missing = [];
  async function relocated(path) {
    if (!located.has(path)) {
      const found = await locate(path, projectFolder);
      located.set(path, found);
      if (!found) missing.push(path);
    }
    return located.get(path) || path;
  }
  async function walk(value) {
    if (typeof value === 'string') return ABSOLUTE_PATH.test(value) ? relocated(value) : value;
    if (Array.isArray(value)) {
      const items = [];
      for (const item of value) items.push(await walk(item));
      return items;
    }
    if (!value || typeof value !== 'object') return value;
    return walkFields(value, textFields);
  }
  async function walkFields(value, skippedKeys) {
    const fields = {};
    for (const [key, field] of Object.entries(value)) {
      fields[key] = skippedKeys.includes(key) ? field : await walk(field);
    }
    return fields;
  }
  return { form: await walkFields(form, [...outputFields, ...textFields]), missing };
}

// === Commands ===

export async function openProject() {
  const picked = await open({ multiple: false, directory: false, defaultPath: await projectFolder(), filters: [projectFilter()] });
  if (picked) await openProjectFile(picked);
}

export async function openProjectFile(path) {
  let text;
  try {
    text = await readTextFile(path);
  } catch (error) {
    if (await existsInScope(path)) {
      configuration.setStatus(`Could not open ${path}: ${error}`);
      return;
    }
    const forget = await confirm(`${path} does not exist. Remove it from recent projects?`, {
      title: 'Project not found',
      kind: 'warning',
    });
    if (forget) removeRecentProject(path);
    return;
  }
  let projectFile;
  try {
    projectFile = readConfiguredProjectFile(text);
  } catch (error) {
    await message(`${path}: ${error.message}`, { title: 'Cannot open project', kind: 'error' });
    return;
  }
  const upgraded = projectFile.version < configuration.projectFileVersion;
  const relocation = await relocateMissingPaths(projectFile.form, folderOf(path), configuration.outputFields, configuration.textFields);
  const notRestored = (await configuration.restore(relocation.form)) || [];
  currentProjectPath = path;
  restoredFromDraft = false;
  projectFileForm = upgraded ? null : JSON.stringify(configuration.serialize());
  await clearDraft();
  addRecentProject(path, recentTitle(relocation.form, path));
  updateWindowTitle();
  const opened = upgraded ? `Opened ${fileNameOf(path)}, upgraded from version ${projectFile.version}` : `Opened ${fileNameOf(path)}`;
  configuration.setStatus(withMissing(opened, [...relocation.missing, ...notRestored]));
}

function withMissing(status, missing) {
  return missing.length ? `${status}, not found: ${missing.join(', ')}` : status;
}

function recentTitle(form, path) {
  return configuration.projectTitle(form) || fileStemOf(path);
}

export async function newProject() {
  const unsaved = JSON.stringify(configuration.serialize()) !== projectFileForm;
  if (unsaved && !(await confirm(DISCARD_CHANGES_QUESTION, { title: 'New project', kind: 'warning' }))) return;
  const name = `${UNTITLED_FILE_STEM}.${extension()}`;
  const picked = await save({ defaultPath: await projectFileSuggestion(name), filters: [projectFilter()] });
  if (!picked) return;
  const path = withExtension(picked);
  await configuration.restore(configuration.defaults);
  configuration.setProjectTitle(fileStemOf(path));
  configuration.setOutputFolder(folderOf(path));
  if (await writeProject(path, configuration.serialize())) return;
  // otherwise Save writes the new form over the old project file
  currentProjectPath = null;
  restoredFromDraft = false;
  projectFileForm = null;
  updateWindowTitle();
}

export async function saveProject() {
  if (!currentProjectPath) {
    await saveProjectAs();
    return;
  }
  await writeProject(currentProjectPath, configuration.serialize());
}

export async function saveProjectAs() {
  const form = configuration.serialize();
  const stem = (configuration.projectTitle(form) || UNTITLED_FILE_STEM).replace(FILE_NAME_UNSAFE, '-');
  const name = `${stem}.${extension()}`;
  const picked = await save({ defaultPath: await projectFileSuggestion(name), filters: [projectFilter()] });
  if (picked) await writeProject(withExtension(picked), form);
}

async function projectFolder() {
  return currentProjectPath ? folderOf(currentProjectPath) : await configuration.defaultProjectFolder();
}

async function projectFileSuggestion(name) {
  const folder = await projectFolder();
  return folder ? joinPath(folder, name) : name;
}

async function writeProjectText(path, form) {
  try {
    await writeTextFile(path, projectFileText(configuration.wizard, configuration.projectFileVersion, form));
  } catch (error) {
    configuration.setStatus(`Could not save ${path}: ${error}`);
    return false;
  }
  return true;
}

async function writeProject(path, form) {
  if (!(await writeProjectText(path, form))) return false;
  await projectSaved(path, form);
  configuration.setStatus(`Saved ${path}`);
  return true;
}

async function projectSaved(path, form) {
  currentProjectPath = path;
  restoredFromDraft = false;
  projectFileForm = JSON.stringify(form);
  addRecentProject(path, recentTitle(form, path));
  await clearDraft(form);
  updateWindowTitle();
}

// also saves the open project file when it is somewhere else
export async function saveProjectBesidePackage(packagePath) {
  const path = projectPathBeside(packagePath);
  const form = configuration.serialize();
  if (!(await writeProjectText(path, form))) return null;
  const openPath = currentProjectPath;
  if (!openPath) return path;
  if (openPath === path || (await writeProjectText(openPath, form))) await projectSaved(openPath, form);
  return path;
}

export async function moveProjectFile(fromPath, toPath, title) {
  try {
    await rename(fromPath, toPath);
  } catch (error) {
    configuration.setStatus(`Could not rename ${fromPath}: ${error}`);
    return;
  }
  if (currentProjectPath === fromPath) currentProjectPath = toPath;
  storeRecentProjects(getRecentProjects().map((entry) => (entry.path === fromPath ? { ...entry, path: toPath, title } : entry)));
  updateWindowTitle();
  await renderRecentProjects();
}

// === Draft ===

async function restoreDraft() {
  if (!(await exists(draftPath))) {
    formOnDisk = JSON.stringify(configuration.serialize());
    return;
  }
  let form;
  let text;
  try {
    text = await readTextFile(draftPath);
    form = readConfiguredProjectFile(text).form;
  } catch (error) {
    configuration.setStatus(`Could not restore the unsaved project in ${draftPath}: ${error.message || error}`);
    formOnDisk = null;
    return;
  }
  const { draftOf } = JSON.parse(text);
  const projectPath = typeof draftOf === 'string' ? draftOf : null;
  const projectFolder = projectPath ? folderOf(projectPath) : null;
  const relocation = await relocateMissingPaths(form, projectFolder, configuration.outputFields, configuration.textFields);
  const notRestored = (await configuration.restore(relocation.form)) || [];
  currentProjectPath = projectPath;
  restoredFromDraft = true;
  projectFileForm = null;
  formOnDisk = JSON.stringify(configuration.serialize());
  updateWindowTitle();
  configuration.setStatus(withMissing('Restored the unsaved project', [...relocation.missing, ...notRestored]));
}

function queueDraftOperation(operation) {
  draftOperations = draftOperations.then(operation, operation);
  return draftOperations;
}

export function saveDraftIfChanged() {
  return queueDraftOperation(async () => {
    if (!draftPath) return;
    const form = configuration.serialize();
    const text = JSON.stringify(form);
    if (text === formOnDisk) return;
    // a failed write is reported once, not every second until the next change
    formOnDisk = text;
    try {
      draftFolderReady ??= mkdir(folderOf(draftPath), { recursive: true });
      await draftFolderReady;
      const draftText = projectFileText(configuration.wizard, configuration.projectFileVersion, form, new Date(), currentProjectPath ?? undefined);
      await writeTextFile(draftPath, draftText);
    } catch (error) {
      draftFolderReady = null;
      configuration.setStatus(`Could not keep the unsaved project in ${draftPath}: ${error}`);
    }
  });
}

function clearDraft(form = configuration.serialize()) {
  return queueDraftOperation(async () => {
    formOnDisk = JSON.stringify(form);
    if (draftPath && (await exists(draftPath))) await remove(draftPath);
  });
}

// === Window title ===

function updateWindowTitle() {
  const fileName = currentProjectPath ? fileNameOf(currentProjectPath) : null;
  const draftMark = restoredFromDraft ? ` ${DRAFT_TITLE_MARK}` : '';
  let title = configuration.applicationName;
  if (fileName) title += `${TITLE_SEPARATOR}${fileName}`;
  title += draftMark;
  if (titleStatus) title += `${TITLE_SEPARATOR}${titleStatus}`;
  document.title = title;
  const projectName = document.getElementById(PROJECT_NAME_ID);
  if (projectName) projectName.textContent = `${currentProjectPath ? fileStemOf(currentProjectPath) : UNTITLED_FILE_STEM}${draftMark}`;
}

export function setWindowTitleStatus(text) {
  titleStatus = text;
  updateWindowTitle();
}

// === Recent projects ===

function recentKey() {
  return `${configuration.wizard}-recent-projects`;
}

function recentCollapsedKey() {
  return `${configuration.wizard}-recent-projects-collapsed`;
}

function recentProjectsCollapsed() {
  return localStorage.getItem(recentCollapsedKey()) !== 'false';
}

function applyRecentProjectsCollapsed() {
  const section = document.getElementById(RECENT_SECTION_ID);
  const toggle = document.getElementById(RECENT_TOGGLE_ID);
  if (!section) return;
  const collapsed = recentProjectsCollapsed();
  section.classList.toggle('collapsed', collapsed);
  if (toggle) {
    toggle.textContent = collapsed ? '▶' : '▼';
    toggle.setAttribute('aria-expanded', String(!collapsed));
  }
}

export function getRecentProjects() {
  try {
    return JSON.parse(localStorage.getItem(recentKey())) || [];
  } catch {
    return [];
  }
}

function storeRecentProjects(entries) {
  localStorage.setItem(recentKey(), JSON.stringify(entries.slice(0, MAX_RECENT)));
}

export function addRecentProject(path, title) {
  storeRecentProjects([{ path, title, time: Date.now() }, ...getRecentProjects().filter((entry) => entry.path !== path)]);
  renderRecentProjects();
}

export function removeRecentProject(path) {
  storeRecentProjects(getRecentProjects().filter((entry) => entry.path !== path));
  renderRecentProjects();
}

// the fs scope refuses paths under a dot folder, which reads here as not there
async function existsInScope(path) {
  try {
    return await exists(path);
  } catch {
    return false;
  }
}

// older entries name a package folder rather than a project file
export async function migrateRecentProjects(entries, fileExtension) {
  const migrated = [];
  for (const entry of entries) {
    if (entry.path.endsWith(`.${fileExtension}`)) {
      migrated.push(entry);
      continue;
    }
    const besidePath = `${entry.path.replace(/[\\/]+$/, '')}.${fileExtension}`;
    if (await existsInScope(besidePath)) migrated.push({ ...entry, path: besidePath });
  }
  return migrated.filter((entry, index) => migrated.findIndex((other) => other.path === entry.path) === index);
}

function rowsInShownOrder(recent) {
  const rank = new Map(shownOrder.map((path, index) => [path, index]));
  const fresh = recent.filter((entry) => !rank.has(entry.path));
  const known = recent.filter((entry) => rank.has(entry.path)).sort((a, b) => rank.get(a.path) - rank.get(b.path));
  const rows = [...fresh, ...known];
  shownOrder = rows.map((entry) => entry.path);
  return rows;
}

function recentRowHtml(entry, packageIsBeside) {
  const noun = configuration.packageNoun;
  const path = escapeHtml(entry.path);
  const packagePath = escapeHtml(packagePathBeside(entry.path));
  const title = escapeHtml(entry.title || fileNameOf(entry.path));
  const packageButtons = packageIsBeside
    ? `
      <button class="recent-queue" title="Add this ${noun} to the playlist">+</button>
      <button class="recent-retitle" title="Give this ${noun} a new content title">✎</button>
      <button class="recent-delete" title="Delete this ${noun} from disk">✕</button>`
    : '';
  return `
    <div class="recent-item" data-path="${path}" data-package-path="${packagePath}" title="${path}">
      <div class="recent-item-text">
        <span class="recent-title">${title}</span>
        <span class="recent-path">${path}</span>
      </div>${packageButtons}
    </div>`;
}

export async function renderRecentProjects() {
  const section = document.getElementById(RECENT_SECTION_ID);
  const list = document.getElementById(RECENT_LIST_ID);
  if (!section || !list) return;
  applyRecentProjectsCollapsed();
  const generation = ++renderGeneration;
  const recent = getRecentProjects();
  const packagesBeside = await Promise.all(recent.map((entry) => existsInScope(packagePathBeside(entry.path))));
  if (generation !== renderGeneration) return;
  if (recent.length === 0) {
    section.hidden = true;
    return;
  }
  section.hidden = false;
  const besideByPath = new Map(recent.map((entry, index) => [entry.path, packagesBeside[index]]));
  list.innerHTML = rowsInShownOrder(recent).map((entry) => recentRowHtml(entry, besideByPath.get(entry.path))).join('');
  wireRecentRows(list);
  configuration.afterRecentRender?.();
}

function wireRecentRows(list) {
  const { onQueue, onRetitle, onDelete, setStatus } = configuration;
  const rowActions = [
    ['.recent-queue', (row) => {
      onQueue(row.dataset.packagePath, row.querySelector('.recent-title').textContent);
      setStatus(`Queued: ${row.dataset.packagePath}`);
    }],
    ['.recent-retitle', (row) => onRetitle(row.dataset.packagePath, row.dataset.path)],
    ['.recent-delete', (row) => onDelete(row.dataset.packagePath)],
  ];
  for (const [selector, action] of rowActions) {
    list.querySelectorAll(selector).forEach((button) => {
      button.addEventListener('click', (event) => {
        event.stopPropagation();
        action(button.closest('.recent-item'));
      });
    });
  }
  list.querySelectorAll('.recent-item').forEach((row) => {
    row.addEventListener('click', () => openProjectFile(row.dataset.path));
  });
}
