import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { watchPreviewSource } from './preview.js';
import {
  DISPLAY_PROFILE_COMMAND,
  displayProfileText,
  notApplicableReasons,
  playerControlCommands,
  playerControlsFromFields,
  playerControlsFromPreferences,
  soundDeviceChoices,
} from './player-controls.js';

const PANEL_ID = 'player-controls';
const DEFAULT_SOUND_DEVICE_TEXT = 'Default';
const DEFAULT_SUBTITLE_COLOUR = '#ffffff';
const SOUND_DEVICES_FAILED_TEXT = 'The sound devices could not be listed';
const BRIGHTNESS_DECIMALS = 2;
const DISPLAY_PROFILE_FILTERS = [{ name: 'ICC profile', extensions: ['icc', 'icm'] }];

const PANEL_MARKUP = `
  <summary class="preview-controls-label">Player</summary>
  <p id="player-control-panel-reason" class="player-controls-reason" hidden></p>
  <fieldset id="player-control-all">
    <div class="preview-controls">
      <span class="preview-controls-label">Picture</span>
      <label>Brightness
        <input type="range" id="player-control-brightness" min="0" max="4" step="0.05">
        <output id="player-control-brightness-value" for="player-control-brightness"></output>
      </label>
      <label title="Percent of the picture each edge hides">Masks
        <input type="number" id="player-control-mask-top" min="0" max="100" step="0.1" title="Top">
        <input type="number" id="player-control-mask-bottom" min="0" max="100" step="0.1" title="Bottom">
        <input type="number" id="player-control-mask-left" min="0" max="100" step="0.1" title="Left">
        <input type="number" id="player-control-mask-right" min="0" max="100" step="0.1" title="Right">
      </label>
      <label>Scaling
        <select id="player-control-scaling">
          <option value="fit">Fit</option>
          <option value="fill">Fill</option>
          <option value="native">Native</option>
        </select>
      </label>
    </div>
    <div class="preview-controls">
      <span class="preview-controls-label">Display</span>
      <fieldset id="player-control-profile-group">
        <span>Monitor profile</span>
        <span id="player-control-profile"></span>
        <button type="button" id="player-control-profile-choose" class="btn-sm">Choose</button>
        <button type="button" id="player-control-profile-clear" class="btn-sm">Built-in</button>
      </fieldset>
      <span id="player-control-profile-reason" class="player-controls-reason" hidden></span>
      <fieldset id="player-control-stereo-group">
        <label>3D
          <select id="player-control-stereo">
            <option value="leftEye">Left eye</option>
            <option value="rightEye">Right eye</option>
            <option value="sideBySide">Side by side</option>
            <option value="topAndBottom">Top and bottom</option>
          </select>
        </label>
      </fieldset>
      <span id="player-control-stereo-reason" class="player-controls-reason" hidden></span>
    </div>
    <div class="preview-controls">
      <span class="preview-controls-label">Sound</span>
      <label>Device <select id="player-control-sound-device"></select></label>
      <span id="player-control-sound-device-error" class="player-controls-error" hidden></span>
      <label>Layout
        <select id="player-control-sound-layout">
          <option value="stereo">Stereo</option>
          <option value="fivePointOne">5.1</option>
          <option value="sevenPointOne">7.1</option>
          <option value="automatic">Automatic</option>
        </select>
      </label>
      <label title="Milliseconds, positive delays the sound">A/V delay
        <input type="number" id="player-control-sound-delay" min="-10000" max="10000" step="1"> ms
      </label>
    </div>
    <div class="preview-controls">
      <span class="preview-controls-label">Subtitles</span>
      <label title="Percent of the height, positive moves up">Offset
        <input type="number" id="player-control-subtitle-offset" min="-50" max="50" step="0.5"> %
      </label>
      <label><input type="checkbox" id="player-control-subtitle-colour-overridden"> Colour</label>
      <input type="color" id="player-control-subtitle-colour">
    </div>
  </fieldset>
  <p id="player-control-error" class="player-controls-error" hidden></p>`;

function element(id) {
  return document.getElementById(id);
}

function choiceOption(value, text) {
  const option = document.createElement('option');
  option.value = value;
  option.textContent = text;
  return option;
}

function showText(target, text) {
  target.textContent = text ?? '';
  target.hidden = !text;
}

// under the QC strip, so the native video over the surface cannot cover it
function addPanel() {
  const previewPanel = element('preview-panel');
  if (!previewPanel || element(PANEL_ID)) return null;
  const panel = document.createElement('details');
  panel.id = PANEL_ID;
  panel.className = 'player-controls';
  panel.innerHTML = PANEL_MARKUP;
  previewPanel.append(panel);
  return panel;
}

function panelFields() {
  return {
    all: element('player-control-all'),
    panelReason: element('player-control-panel-reason'),
    brightness: element('player-control-brightness'),
    brightnessValue: element('player-control-brightness-value'),
    maskTop: element('player-control-mask-top'),
    maskBottom: element('player-control-mask-bottom'),
    maskLeft: element('player-control-mask-left'),
    maskRight: element('player-control-mask-right'),
    scaling: element('player-control-scaling'),
    profileGroup: element('player-control-profile-group'),
    profile: element('player-control-profile'),
    profileReason: element('player-control-profile-reason'),
    stereoGroup: element('player-control-stereo-group'),
    stereo: element('player-control-stereo'),
    stereoReason: element('player-control-stereo-reason'),
    soundDevice: element('player-control-sound-device'),
    soundDeviceError: element('player-control-sound-device-error'),
    soundLayout: element('player-control-sound-layout'),
    soundDelayMilliseconds: element('player-control-sound-delay'),
    subtitleOffsetPercent: element('player-control-subtitle-offset'),
    subtitleColourOverridden: element('player-control-subtitle-colour-overridden'),
    subtitleColour: element('player-control-subtitle-colour'),
    error: element('player-control-error'),
  };
}

function showProfile(fields, profile) {
  fields.profile.dataset.path = profile ?? '';
  fields.profile.textContent = displayProfileText(profile);
  fields.profile.title = profile ?? '';
}

function showBrightness(fields) {
  fields.brightnessValue.value = Number(fields.brightness.value).toFixed(BRIGHTNESS_DECIMALS);
}

function enableSubtitleColour(fields) {
  fields.subtitleColour.disabled = !fields.subtitleColourOverridden.checked;
}

// a device list that fails leaves the saved device and says why
async function fillSoundDevices(fields, savedDevice) {
  const listing = await invoke('preview_sound_devices').then(
    (names) => ({ names, error: null }),
    (error) => ({ names: [], error: `${SOUND_DEVICES_FAILED_TEXT}: ${error}` }),
  );
  fields.soundDevice.replaceChildren(
    choiceOption('', DEFAULT_SOUND_DEVICE_TEXT),
    ...soundDeviceChoices(listing.names, savedDevice).map((device) => choiceOption(device.name, device.label)),
  );
  fields.soundDevice.value = savedDevice ?? '';
  showText(fields.soundDeviceError, listing.error);
}

function fillFields(fields, controls) {
  const { playerPicture, playerSound, playerSubtitles, playerDisplayProfile, playerStereo } = controls;
  fields.brightness.value = playerPicture.brightness;
  fields.maskTop.value = playerPicture.masksPercent.top;
  fields.maskBottom.value = playerPicture.masksPercent.bottom;
  fields.maskLeft.value = playerPicture.masksPercent.left;
  fields.maskRight.value = playerPicture.masksPercent.right;
  fields.scaling.value = playerPicture.scaling;
  showProfile(fields, playerDisplayProfile);
  fields.stereo.value = playerStereo;
  fields.soundLayout.value = playerSound.layout;
  fields.soundDelayMilliseconds.value = playerSound.delayMilliseconds;
  fields.subtitleOffsetPercent.value = playerSubtitles.offsetPercent;
  fields.subtitleColourOverridden.checked = playerSubtitles.colour !== null;
  fields.subtitleColour.value = playerSubtitles.colour ?? DEFAULT_SUBTITLE_COLOUR;
  showBrightness(fields);
  enableSubtitleColour(fields);
}

function controlsFromFields(fields) {
  return playerControlsFromFields({
    brightness: fields.brightness.value,
    maskTop: fields.maskTop.value,
    maskBottom: fields.maskBottom.value,
    maskLeft: fields.maskLeft.value,
    maskRight: fields.maskRight.value,
    scaling: fields.scaling.value,
    soundDevice: fields.soundDevice.value,
    soundLayout: fields.soundLayout.value,
    soundDelayMilliseconds: fields.soundDelayMilliseconds.value,
    subtitleOffsetPercent: fields.subtitleOffsetPercent.value,
    subtitleColourOverridden: fields.subtitleColourOverridden.checked,
    subtitleColour: fields.subtitleColour.value,
    displayProfile: fields.profile.dataset.path,
    stereo: fields.stereo.value,
  });
}

async function showApplicability(fields) {
  const reasons = notApplicableReasons(await invoke('preview_loaded_picture'));
  showText(fields.panelReason, reasons.panel);
  fields.all.disabled = reasons.panel !== null;
  showText(fields.profileReason, reasons.displayProfile);
  fields.profileGroup.disabled = reasons.displayProfile !== null;
  showText(fields.stereoReason, reasons.stereo);
  fields.stereoGroup.disabled = reasons.stereo !== null;
}

// the screening controls under the preview, applied live and kept in the app's preferences through save
export async function initPlayerControlsPanel({ preferences, save }) {
  if (!addPanel()) return;
  const fields = panelFields();
  const saved = playerControlsFromPreferences(preferences);
  fillFields(fields, saved);
  await fillSoundDevices(fields, saved.playerSound.device);

  let applied = null;
  // changes apply one after another, so a slow command cannot land after a later one
  let applying = Promise.resolve();

  // a refused profile leaves the one the player has and says why
  async function applyFromFields() {
    let controls = controlsFromFields(fields);
    showText(fields.error, null);
    for (const [command, args] of playerControlCommands(applied, controls)) {
      const refusal = await invoke(command, args).then(() => null, String);
      if (!refusal) continue;
      showText(fields.error, refusal);
      if (command !== DISPLAY_PROFILE_COMMAND) continue;
      controls = { ...controls, playerDisplayProfile: applied?.playerDisplayProfile ?? null };
      showProfile(fields, controls.playerDisplayProfile);
    }
    applied = controls;
  }

  const applyChanges = () => {
    applying = applying.then(applyFromFields);
    return applying;
  };
  const applyAndSave = () => applyChanges().then(() => save(applied));

  await applyChanges();
  fields.all.addEventListener('change', applyAndSave);
  fields.brightness.addEventListener('input', () => {
    showBrightness(fields);
    applyChanges();
  });
  fields.subtitleColourOverridden.addEventListener('change', () => enableSubtitleColour(fields));
  element('player-control-profile-choose').addEventListener('click', async () => {
    const profile = await open({ filters: DISPLAY_PROFILE_FILTERS });
    if (!profile) return;
    showProfile(fields, profile);
    await applyAndSave();
  });
  element('player-control-profile-clear').addEventListener('click', () => {
    showProfile(fields, null);
    applyAndSave();
  });
  const refreshApplicability = () => showApplicability(fields).catch((error) => showText(fields.error, String(error)));
  watchPreviewSource(refreshApplicability);
  await refreshApplicability();
}
