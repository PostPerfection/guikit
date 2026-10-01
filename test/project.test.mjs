import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-plugins-hooks.mjs', import.meta.url);

const tauri = await import('./tauri-plugins-stub.mjs');
const project = await import('../src/project.js');

const WIZARD = 'dcpwizard';
const APPLICATION_NAME = 'DCP Wizard';
const DRAFT_PATH = `${tauri.APP_DATA_FOLDER}/draft.dcpwizard`;
const RECENT_KEY = 'dcpwizard-recent-projects';
const DEFAULT_PROJECT_FOLDER = '/home/u/Documents';

function fakeElement() {
  return {
    hidden: false,
    innerHTML: '',
    textContent: '',
    handlers: {},
    classList: { toggle() {} },
    setAttribute() {},
    addEventListener(name, handler) {
      this.handlers[name] = handler;
    },
    querySelectorAll() {
      return [];
    },
  };
}

const elements = new Map();
globalThis.document = {
  title: '',
  getElementById(id) {
    if (!elements.has(id)) elements.set(id, fakeElement());
    return elements.get(id);
  },
};

const storage = new Map();
globalThis.localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => storage.set(key, String(value)),
};

// the draft is written by calling saveDraftIfChanged, not by waiting on a timer
globalThis.setInterval = () => 0;

let panel;
let statuses;

function panelHolding(form) {
  const state = { form: structuredClone(form), restored: [] };
  return {
    state,
    options: {
      wizard: WIZARD,
      applicationName: APPLICATION_NAME,
      packageNoun: 'DCP',
      outputFields: ['keyOut'],
      textFields: ['title'],
      defaults: structuredClone(form),
      defaultProjectFolder: async () => DEFAULT_PROJECT_FOLDER,
      setProjectTitle: (title) => {
        state.form.title = title;
      },
      setOutputFolder: (folder) => {
        state.form.outputDir = folder;
      },
      serialize: () => structuredClone(state.form),
      restore: async (form) => {
        state.restored.push(form);
        state.form = structuredClone(form);
        return [];
      },
      projectTitle: (form) => form.title,
      onQueue() {},
      onRetitle() {},
      onDelete() {},
      setStatus: (text) => statuses.push(text),
    },
  };
}

async function launch(form, diskEntries = {}, recent = []) {
  tauri.resetDisk(diskEntries);
  storage.clear();
  storage.set(RECENT_KEY, JSON.stringify(recent));
  statuses = [];
  panel = panelHolding(form);
  await project.initProjects(panel.options);
}

function projectText(form, wizard = WIZARD) {
  return project.projectFileText(wizard, form);
}

function recentPaths() {
  return JSON.parse(storage.get(RECENT_KEY)).map((entry) => entry.path);
}

test('a project file carries the wizard, the version and the form, and reads back to the form', () => {
  const form = { title: 'Film', project: { assets: [{ path: '/media/film.mov' }] } };
  const saved = JSON.parse(projectText(form));

  assert.equal(saved.wizard, WIZARD);
  assert.equal(saved.version, project.PROJECT_FILE_VERSION);
  assert.ok(!Number.isNaN(Date.parse(saved.saved)));
  assert.deepEqual(project.readProjectFile(projectText(form), WIZARD), form);
});

test('a file from another wizard is refused naming that wizard', () => {
  assert.throws(() => project.readProjectFile(projectText({}, 'imfwizard'), WIZARD), /imfwizard/);
});

test('a file from a newer project version is refused naming the version', () => {
  const newer = JSON.stringify({ wizard: WIZARD, version: project.PROJECT_FILE_VERSION + 1, form: {} });
  assert.throws(() => project.readProjectFile(newer, WIZARD), /version 2/);
});

test('a file with no form reads as an empty form, so every field takes its default', () => {
  const bare = JSON.stringify({ wizard: WIZARD, version: 1 });
  assert.deepEqual(project.readProjectFile(bare, WIZARD), {});
});

test('a version below 1 is refused as not a project file', () => {
  for (const version of [0, -1]) {
    const text = JSON.stringify({ wizard: WIZARD, version, form: {} });
    assert.throws(() => project.readProjectFile(text, WIZARD), /not a project file/, `version ${version}`);
  }
});

test('text that is not JSON is refused as not a project file', () => {
  assert.throws(() => project.readProjectFile('<xml/>', WIZARD), /not a project file/);
});

test('a missing path is found again beside the project file', async () => {
  tauri.resetDisk({
    '/moved/film/media/clip.mov': '',
    '/moved/film/music.wav': '',
    '/moved/film': null,
    '/still/here.srt': '',
  });
  const form = {
    title: 'A title, not a path',
    outputDir: '/old/work/film',
    keyOut: '/old/work/keys.json',
    project: {
      assets: [
        { path: '/old/work/film/media/clip.mov' },
        { path: '/somewhere/else/music.wav' },
        { path: '/still/here.srt' },
        { path: '/old/work/gone.xml' },
      ],
    },
  };

  const { form: relocated, missing } = await project.relocateMissingPaths(form, '/moved/film', ['keyOut']);

  assert.equal(relocated.title, form.title);
  assert.equal(relocated.outputDir, '/moved/film', 'a folder named like the project folder is that folder');
  assert.equal(relocated.keyOut, form.keyOut, 'a file the build writes is not looked for');
  assert.deepEqual(relocated.project.assets.map((asset) => asset.path), [
    '/moved/film/media/clip.mov',
    '/moved/film/music.wav',
    '/still/here.srt',
    '/old/work/gone.xml',
  ]);
  assert.deepEqual(missing, ['/old/work/gone.xml']);
});

test('a title that starts with a slash stays as typed while the paths beside it are found again', async () => {
  const saved = { title: '/Part 2/Final', project: { title: '/Part 2/Final', assets: [{ path: '/old/Final' }] } };
  await launch({ title: '' }, {
    '/home/u/proj/Film.dcpwizard': projectText(saved),
    '/home/u/proj/Final': '',
  });

  await project.openProjectFile('/home/u/proj/Film.dcpwizard');

  assert.deepEqual(panel.state.restored.at(-1), {
    title: '/Part 2/Final',
    project: { title: '/Part 2/Final', assets: [{ path: '/home/u/proj/Final' }] },
  });
  assert.equal(statuses.at(-1), 'Opened Film.dcpwizard');
});

test('the longest tail is tried first, so a nested copy wins over a same-named file at the top', () => {
  assert.deepEqual(project.relocationCandidates('/old/film/media/clip.mov', '/moved/film'), [
    '/moved/film/media/clip.mov',
    '/moved/film/film/media/clip.mov',
    '/moved/film/clip.mov',
  ]);
});

test('Windows paths are relocated with the project folder separator', () => {
  assert.deepEqual(project.relocationCandidates('D:\\old\\clip.mov', 'E:\\moved'), [
    'E:\\moved\\old\\clip.mov',
    'E:\\moved\\clip.mov',
  ]);
});

test('an old package entry moves to the project file beside it, or goes when there is none', async () => {
  tauri.resetDisk({ '/out/Film.dcpwizard': '' });
  const migrated = await project.migrateRecentProjects(
    [
      { path: '/out/Film/', title: 'Film', time: 3 },
      { path: '/out/Trailer', title: 'Trailer', time: 2 },
      { path: '/projects/Short.dcpwizard', title: 'Short', time: 1 },
      { path: '/out/Film', title: 'Film again', time: 0 },
    ],
    WIZARD,
  );
  assert.deepEqual(migrated, [
    { path: '/out/Film.dcpwizard', title: 'Film', time: 3 },
    { path: '/projects/Short.dcpwizard', title: 'Short', time: 1 },
  ]);
});

test('launch migrates the stored recent list', async () => {
  await launch({ title: '' }, { '/out/Film.dcpwizard': '' }, [
    { path: '/out/Film', title: 'Film', time: 1 },
    { path: '/out/Gone', title: 'Gone', time: 0 },
  ]);
  assert.deepEqual(recentPaths(), ['/out/Film.dcpwizard']);
});

test('opening a project replaces the form, names the missing paths and titles the window', async () => {
  const saved = { title: 'Film', source: '/old/film/clip.mov', subtitle: '/old/film/gone.srt' };
  await launch({ title: '' }, {
    '/moved/film/Film.dcpwizard': projectText(saved),
    '/moved/film/clip.mov': '',
    [DRAFT_PATH]: projectText({ title: 'Draft' }),
  });
  tauri.answerDialog('open', '/moved/film/Film.dcpwizard');

  await elements.get('btn-project-open').handlers.click();

  assert.deepEqual(panel.state.restored.at(-1), {
    title: 'Film',
    source: '/moved/film/clip.mov',
    subtitle: '/old/film/gone.srt',
  });
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
  assert.match(statuses.at(-1), /not found: \/old\/film\/gone\.srt$/);
  assert.equal(recentPaths()[0], '/moved/film/Film.dcpwizard');
  assert.equal(tauri.disk.has(DRAFT_PATH), false, 'the opened file replaces the draft');
});

test('opening another wizard\'s file shows the refusal and leaves the form alone', async () => {
  await launch({ title: 'Kept' }, { '/p/Imf.dcpwizard': projectText({ title: 'Imf' }, 'imfwizard') });
  await project.openProjectFile('/p/Imf.dcpwizard');

  assert.equal(panel.state.restored.length, 0);
  assert.match(tauri.messages[0][0], /imfwizard/);
  assert.equal(panel.state.form.title, 'Kept');
});

test('Save As adds the extension and Save then writes to the same file without asking', async () => {
  await launch({ title: 'Film', source: '/media/film.mov' });
  tauri.answerDialog('save', '/projects/Film');

  await elements.get('btn-project-save-as').handlers.click();

  assert.equal(tauri.dialogRequests.save.at(-1).defaultPath, 'Film.dcpwizard');
  assert.deepEqual(project.readProjectFile(tauri.disk.get('/projects/Film.dcpwizard'), WIZARD), panel.state.form);
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
  assert.equal(recentPaths()[0], '/projects/Film.dcpwizard');

  const asked = tauri.dialogRequests.save.length;
  panel.state.form.title = 'Film, recut';
  await elements.get('btn-project-save').handlers.click();

  assert.equal(tauri.dialogRequests.save.length, asked);
  assert.equal(project.readProjectFile(tauri.disk.get('/projects/Film.dcpwizard'), WIZARD).title, 'Film, recut');
});

test('Save with no project file asks where to save it', async () => {
  await launch({ title: 'New' });
  await elements.get('btn-project-save').handlers.click();
  assert.equal(tauri.dialogRequests.save.at(-1).defaultPath, 'New.dcpwizard');
});

test('the file beside a package is a record, the open project stays the one Save writes to', async () => {
  await launch({ title: 'Film' });
  tauri.answerDialog('save', '/projects/Film.dcpwizard');
  await project.saveProjectAs();

  const besidePath = await project.saveProjectBesidePackage('/out/Film_FTR');

  assert.equal(besidePath, '/out/Film_FTR.dcpwizard');
  assert.deepEqual(project.readProjectFile(tauri.disk.get(besidePath), WIZARD), panel.state.form);
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
});

test('a changed form is kept as a draft, restored on the next launch and cleared by a new project', async () => {
  await launch({ title: '' });
  await project.saveDraftIfChanged();
  assert.equal(tauri.disk.has(DRAFT_PATH), false, 'an untouched form writes no draft');

  panel.state.form.title = 'Unsaved';
  await project.saveDraftIfChanged();
  const draft = tauri.disk.get(DRAFT_PATH);
  assert.equal(project.readProjectFile(draft, WIZARD).title, 'Unsaved');

  await launch({ title: '' }, { [DRAFT_PATH]: draft });
  assert.equal(panel.state.form.title, 'Unsaved');
  assert.equal(document.title, `${APPLICATION_NAME} (draft)`);

  tauri.answerDialog('confirm', true);
  tauri.answerDialog('save', '/projects/Next.dcpwizard');
  await project.newProject();
  assert.equal(tauri.disk.has(DRAFT_PATH), false);
  assert.equal(document.title, `${APPLICATION_NAME} - Next.dcpwizard`);
});

test('closing the window writes the draft', async () => {
  await launch({ title: '' });
  panel.state.form.title = 'Closing';
  await tauri.closeHandler();
  assert.equal(project.readProjectFile(tauri.disk.get(DRAFT_PATH), WIZARD).title, 'Closing');
});

test('saving clears the draft', async () => {
  await launch({ title: 'Film' }, { [DRAFT_PATH]: projectText({ title: 'Film' }) });
  tauri.answerDialog('save', '/projects/Film.dcpwizard');
  await project.saveProject();
  assert.equal(tauri.disk.has(DRAFT_PATH), false);
});

test('the title keeps the build progress beside the project file name', async () => {
  await launch({ title: 'Film' });
  tauri.answerDialog('save', '/projects/Film.dcpwizard');
  await project.saveProjectAs();

  project.setWindowTitleStatus('Encoding 42%');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard - Encoding 42%`);
  project.setWindowTitleStatus('');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
});

test('a recent row offers the package actions only when the package is beside the file', async () => {
  await launch(
    { title: '' },
    { '/out/Film.dcpwizard': '', '/out/Film': null, '/out/Gone.dcpwizard': '' },
    [
      { path: '/out/Film.dcpwizard', title: 'Film', time: 1 },
      { path: '/out/Gone.dcpwizard', title: 'Gone', time: 0 },
    ],
  );
  const rows = elements.get('recent-list').innerHTML.split('class="recent-item"').slice(1);

  assert.equal(rows.length, 2);
  assert.match(rows[0], /data-package-path="\/out\/Film"/);
  assert.match(rows[0], /recent-queue/);
  assert.doesNotMatch(rows[1], /recent-queue|recent-retitle|recent-delete/);
});

test('a project file passed at launch is opened in place of the draft', async () => {
  tauri.answerCommand('take_launch_project_path', '/p/Film.dcpwizard');
  await launch({ title: '' }, {
    '/p/Film.dcpwizard': projectText({ title: 'Film' }),
    [DRAFT_PATH]: projectText({ title: 'Draft' }),
  });

  assert.equal(panel.state.form.title, 'Film');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
  assert.equal(recentPaths()[0], '/p/Film.dcpwizard');
  assert.equal(tauri.disk.has(DRAFT_PATH), false, 'the opened file replaces the draft');
});

test('a project file opened while the app runs replaces the form', async () => {
  await launch({ title: 'Kept' }, { '/p/Short.dcpwizard': projectText({ title: 'Short' }) });
  assert.equal(panel.state.form.title, 'Kept', 'no launch path opens nothing');

  await tauri.eventListeners.get('open-project-file')({ event: 'open-project-file', payload: '/p/Short.dcpwizard' });

  assert.equal(panel.state.form.title, 'Short');
  assert.equal(document.title, `${APPLICATION_NAME} - Short.dcpwizard`);
});

test('New with nothing changed asks nothing, then saves a default form named after the file', async () => {
  await launch({ title: '', outputDir: '', source: '' });
  const asked = tauri.dialogRequests.confirm.length;
  tauri.answerDialog('save', '/projects/Film');

  await elements.get('btn-new-project').handlers.click();

  assert.equal(tauri.dialogRequests.confirm.length, asked);
  assert.equal(tauri.dialogRequests.save.at(-1).defaultPath, `${DEFAULT_PROJECT_FOLDER}/Untitled.dcpwizard`);
  const saved = { title: 'Film', outputDir: '/projects', source: '' };
  assert.deepEqual(panel.state.form, saved);
  assert.deepEqual(project.readProjectFile(tauri.disk.get('/projects/Film.dcpwizard'), WIZARD), saved);
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
  assert.equal(recentPaths()[0], '/projects/Film.dcpwizard');
});

test('New over unsaved changes asks first, and No or a cancelled save leaves the form alone', async () => {
  await launch({ title: '' }, { '/p/Film.dcpwizard': projectText({ title: 'Film', source: '/p/clip.mov' }), '/p/clip.mov': '' });
  await project.openProjectFile('/p/Film.dcpwizard');
  const asked = tauri.dialogRequests.confirm.length;
  tauri.answerDialog('save', null);
  await project.newProject();
  assert.equal(tauri.dialogRequests.confirm.length, asked, 'the form matches the open file');
  assert.equal(tauri.dialogRequests.save.at(-1).defaultPath, '/p/Untitled.dcpwizard');

  panel.state.form.source = '/p/other.mov';
  const saveDialogs = tauri.dialogRequests.save.length;
  tauri.answerDialog('confirm', false);
  await project.newProject();
  assert.equal(tauri.dialogRequests.confirm.at(-1), 'Discard unsaved changes?');
  assert.equal(tauri.dialogRequests.save.length, saveDialogs, 'No asks for no file');

  tauri.answerDialog('confirm', true);
  tauri.answerDialog('save', null);
  await project.newProject();
  assert.equal(panel.state.form.source, '/p/other.mov');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
});

test('a build saves the open project as well as the file beside the package', async () => {
  await launch({ title: 'Film' });
  tauri.answerDialog('save', '/projects/Film.dcpwizard');
  await project.saveProjectAs();
  panel.state.form.title = 'Film, recut';
  await project.saveDraftIfChanged();

  await project.saveProjectBesidePackage('/out/Film_FTR');

  for (const path of ['/out/Film_FTR.dcpwizard', '/projects/Film.dcpwizard']) {
    assert.equal(project.readProjectFile(tauri.disk.get(path), WIZARD).title, 'Film, recut', path);
  }
  assert.equal(tauri.disk.has(DRAFT_PATH), false);
});

test('a build whose package sits beside the open project writes that file once', async () => {
  await launch({ title: '' }, { '/out/Film.dcpwizard': projectText({ title: 'Film' }) });
  await project.openProjectFile('/out/Film.dcpwizard');
  const writesBefore = tauri.writes.length;

  assert.equal(await project.saveProjectBesidePackage('/out/Film'), '/out/Film.dcpwizard');

  assert.deepEqual(tauri.writes.slice(writesBefore), ['/out/Film.dcpwizard']);
});

test('a draft remembers its project file, so the next launch titles it and Save writes there', async () => {
  await launch({ title: '' }, { '/p/Film.dcpwizard': projectText({ title: 'Film' }) });
  await project.openProjectFile('/p/Film.dcpwizard');
  panel.state.form.title = 'Film, recut';
  await project.saveDraftIfChanged();
  const draft = tauri.disk.get(DRAFT_PATH);
  assert.equal(JSON.parse(draft).draftOf, '/p/Film.dcpwizard');
  assert.equal(JSON.parse(tauri.disk.get('/p/Film.dcpwizard')).draftOf, undefined, 'a project file has no draftOf');

  await launch({ title: '' }, { '/p/Film.dcpwizard': projectText({ title: 'Film' }), [DRAFT_PATH]: draft });
  assert.equal(panel.state.form.title, 'Film, recut');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard (draft)`);

  const saveDialogs = tauri.dialogRequests.save.length;
  await project.saveProject();
  assert.equal(tauri.dialogRequests.save.length, saveDialogs);
  assert.equal(project.readProjectFile(tauri.disk.get('/p/Film.dcpwizard'), WIZARD).title, 'Film, recut');
  assert.equal(document.title, `${APPLICATION_NAME} - Film.dcpwizard`);
});
