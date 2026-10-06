import { invoke } from '@tauri-apps/api/core';

const GPU_CHECKBOX_ID = 'set-gpu';
const LICENSE_INPUT_ID = 'set-gpu-license';
const LICENSE_SHOW_BUTTON_ID = 'set-gpu-license-show';
const REGISTRATION_URL_INPUT_ID = 'set-gpu-registration-url';
const GPU_NOT_ENABLED = 'Grok did not enable the GPU';

export function initGpuSettings() {
  document.getElementById(LICENSE_SHOW_BUTTON_ID)?.addEventListener('click', (event) => {
    const license = document.getElementById(LICENSE_INPUT_ID);
    const hidden = license.type === 'password';
    license.type = hidden ? 'text' : 'password';
    event.currentTarget.textContent = hidden ? 'Hide' : 'Show';
  });
}

export function fillGpuSettings({ gpu, gpuLicense, gpuRegistrationUrl }) {
  const checkbox = document.getElementById(GPU_CHECKBOX_ID);
  if (checkbox) checkbox.checked = gpu;
  const license = document.getElementById(LICENSE_INPUT_ID);
  if (license) license.value = gpuLicense ?? '';
  const registrationUrl = document.getElementById(REGISTRATION_URL_INPUT_ID);
  if (registrationUrl) registrationUrl.value = gpuRegistrationUrl ?? '';
}

export function gpuSettingsFromForm() {
  return {
    gpu: !!document.getElementById(GPU_CHECKBOX_ID)?.checked,
    gpuLicense: document.getElementById(LICENSE_INPUT_ID)?.value.trim() || '',
    gpuRegistrationUrl: document.getElementById(REGISTRATION_URL_INPUT_ID)?.value.trim() || '',
  };
}

export function uncheckGpu() {
  const checkbox = document.getElementById(GPU_CHECKBOX_ID);
  if (checkbox) checkbox.checked = false;
}

// null once grok runs as asked, otherwise why the GPU is not running
export async function applyGpuSetting({ gpu, gpuLicense, gpuRegistrationUrl }, encodeThreads = null) {
  try {
    const active = await invoke('set_gpu', {
      enabled: gpu,
      license: gpuLicense || null,
      registrationUrl: gpuRegistrationUrl || null,
      encodeThreads,
    });
    return gpu && !active ? GPU_NOT_ENABLED : null;
  } catch (error) {
    return String(error);
  }
}
