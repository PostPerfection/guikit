import assert from 'node:assert/strict';
import { register } from 'node:module';
import test from 'node:test';

register('./tauri-core-hooks.mjs', import.meta.url);

const bridge = await import('./tauri-core-stub.mjs');

const LICENSE = 'licence-token';
const REGISTRATION_URL = 'https://licence.example/register';
const ENCODE_THREADS = 8;
const DEVICE_ERROR = 'no CUDA device';

function fakeInput(type) {
  const listeners = {};
  return {
    type,
    value: '',
    checked: false,
    textContent: 'Show',
    addEventListener(name, handler) {
      listeners[name] = handler;
    },
    click() {
      listeners.click({ currentTarget: this });
    },
  };
}

const checkbox = fakeInput('checkbox');
const license = fakeInput('password');
const showButton = fakeInput('button');
const registrationUrl = fakeInput('url');
const elements = new Map([
  ['set-gpu', checkbox],
  ['set-gpu-license', license],
  ['set-gpu-license-show', showButton],
  ['set-gpu-registration-url', registrationUrl],
]);
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

const gpuSettings = await import('../src/gpu-settings.js');
gpuSettings.initGpuSettings();

function setGpuRequests() {
  return bridge.invocations.filter(([command]) => command === 'set_gpu').map(([, args]) => args);
}

test.beforeEach(() => {
  bridge.forgetInvocations();
  bridge.forgetRefusals();
});

test('the Show button reveals the licence and hides it again', () => {
  showButton.click();
  assert.equal(license.type, 'text');
  assert.equal(showButton.textContent, 'Hide');

  showButton.click();
  assert.equal(license.type, 'password');
  assert.equal(showButton.textContent, 'Show');
});

test('the form reads back what it was filled with, trimmed', () => {
  gpuSettings.fillGpuSettings({ gpu: true, gpuLicense: ` ${LICENSE} `, gpuRegistrationUrl: REGISTRATION_URL });
  assert.deepEqual(gpuSettings.gpuSettingsFromForm(), {
    gpu: true,
    gpuLicense: LICENSE,
    gpuRegistrationUrl: REGISTRATION_URL,
  });
});

test('unset licence fields fill as empty and unchecking clears the box', () => {
  gpuSettings.fillGpuSettings({ gpu: true, gpuLicense: null, gpuRegistrationUrl: null });
  assert.equal(license.value, '');
  assert.equal(registrationUrl.value, '');

  gpuSettings.uncheckGpu();
  assert.equal(checkbox.checked, false);
});

test('applying sends the setting with empty fields as null and succeeds when the GPU is active', async () => {
  bridge.answerWith('set_gpu', true);
  const failure = await gpuSettings.applyGpuSetting(
    { gpu: true, gpuLicense: LICENSE, gpuRegistrationUrl: '' },
    ENCODE_THREADS,
  );
  assert.equal(failure, null);
  assert.deepEqual(setGpuRequests(), [
    { enabled: true, license: LICENSE, registrationUrl: null, encodeThreads: ENCODE_THREADS },
  ]);
});

test('an app that does not encode sends no encode threads', async () => {
  bridge.answerWith('set_gpu', false);
  const failure = await gpuSettings.applyGpuSetting({ gpu: false, gpuLicense: '', gpuRegistrationUrl: '' });
  assert.equal(failure, null);
  assert.deepEqual(setGpuRequests(), [
    { enabled: false, license: null, registrationUrl: null, encodeThreads: null },
  ]);
});

test('asking for the GPU fails when grok leaves it off', async () => {
  bridge.answerWith('set_gpu', false);
  const failure = await gpuSettings.applyGpuSetting({ gpu: true, gpuLicense: LICENSE, gpuRegistrationUrl: '' });
  assert.equal(failure, 'Grok did not enable the GPU');
});

test('a refused request reports the backend error', async () => {
  bridge.refuseWith('set_gpu', DEVICE_ERROR);
  const failure = await gpuSettings.applyGpuSetting({ gpu: true, gpuLicense: LICENSE, gpuRegistrationUrl: '' });
  assert.equal(failure, DEVICE_ERROR);
});
