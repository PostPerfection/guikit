import assert from 'node:assert/strict';
import test from 'node:test';

import {
  DISPLAY_PROFILE_COMMAND,
  displayProfileText,
  notApplicableReasons,
  playerControlCommands,
  playerControlsFromFields,
  playerControlsFromPreferences,
  soundDeviceChoices,
} from '../src/player-controls.js';

const PROFILE = '/usr/share/color/icc/booth.icc';
const SAVED_DEVICE = 'HDMI';

const FIELDS = {
  brightness: '1.25',
  maskTop: '10',
  maskBottom: '0',
  maskLeft: '2.5',
  maskRight: '0',
  scaling: 'fill',
  soundDevice: '',
  soundLayout: 'fivePointOne',
  soundDelayMilliseconds: '-40.4',
  subtitleOffsetPercent: '8',
  subtitleColourOverridden: false,
  subtitleColour: '#ffcc00',
  displayProfile: '',
  stereo: 'sideBySide',
};

test('a preferences file without the controls gives the player defaults, and stored ones are kept', () => {
  const defaults = playerControlsFromPreferences({ gpu: true });
  assert.deepEqual(defaults, {
    playerPicture: { brightness: 1, masksPercent: { top: 0, bottom: 0, left: 0, right: 0 }, scaling: 'fit' },
    playerSound: { device: null, layout: 'stereo', delayMilliseconds: 0 },
    playerSubtitles: { offsetPercent: 0, colour: null },
    playerDisplayProfile: null,
    playerStereo: 'leftEye',
  });

  const stored = playerControlsFromPreferences({ gpu: true, playerDisplayProfile: PROFILE, playerStereo: 'rightEye' });
  assert.equal(stored.playerDisplayProfile, PROFILE);
  assert.equal(stored.playerStereo, 'rightEye');
  assert.equal(stored.gpu, undefined, 'only the controls are taken');
});

test('the fields read as the controls the commands take', () => {
  assert.deepEqual(playerControlsFromFields(FIELDS), {
    playerPicture: { brightness: 1.25, masksPercent: { top: 10, bottom: 0, left: 2.5, right: 0 }, scaling: 'fill' },
    playerSound: { device: null, layout: 'fivePointOne', delayMilliseconds: -40 },
    playerSubtitles: { offsetPercent: 8, colour: null },
    playerDisplayProfile: null,
    playerStereo: 'sideBySide',
  });

  const overridden = playerControlsFromFields({ ...FIELDS, subtitleColourOverridden: true, displayProfile: PROFILE });
  assert.equal(overridden.playerSubtitles.colour, '#ffcc00');
  assert.equal(overridden.playerDisplayProfile, PROFILE);
});

test('nothing applied yet sends every control, after that only what changed', () => {
  const controls = playerControlsFromFields(FIELDS);
  assert.deepEqual(
    playerControlCommands(null, controls).map(([command]) => command),
    [
      'preview_set_picture',
      'preview_set_sound_device',
      'preview_set_sound_layout',
      'preview_set_sound_delay',
      'preview_set_subtitle_presentation',
      'preview_set_stereo_output',
      DISPLAY_PROFILE_COMMAND,
    ],
  );

  const delayed = playerControlsFromFields({ ...FIELDS, soundDelayMilliseconds: '120' });
  assert.deepEqual(playerControlCommands(controls, delayed), [['preview_set_sound_delay', { milliseconds: 120 }]]);
  assert.deepEqual(playerControlCommands(controls, controls), []);
});

test('a saved sound device the system no longer lists stays a choice', () => {
  assert.deepEqual(soundDeviceChoices(['Speakers'], SAVED_DEVICE), [
    { name: 'Speakers', label: 'Speakers' },
    { name: SAVED_DEVICE, label: `${SAVED_DEVICE} (not connected)` },
  ]);
  assert.deepEqual(soundDeviceChoices([SAVED_DEVICE], SAVED_DEVICE), [{ name: SAVED_DEVICE, label: SAVED_DEVICE }]);
});

test('the profile shows as its file name, or the built-in sRGB without one', () => {
  assert.equal(displayProfileText(PROFILE), 'booth.icc');
  assert.equal(displayProfileText('C:\\Profiles\\booth.icm'), 'booth.icm');
  assert.equal(displayProfileText(null), 'Built-in sRGB');
});

test('an IMF picture marks the monitor profile and 3D not applicable', () => {
  const reasons = notApplicableReasons({ kind: 'imf' });
  assert.equal(reasons.panel, null);
  assert.match(reasons.displayProfile, /Not applicable to IMF/);
  assert.match(reasons.stereo, /Not applicable to IMF/);
});

test('a file on mpv marks the whole panel not applicable', () => {
  assert.match(notApplicableReasons({ kind: 'mpvFile' }).panel, /plays on mpv/);
});

test('3D applies to a stereoscopic DCP only, and everything applies with nothing loaded', () => {
  assert.deepEqual(notApplicableReasons({ kind: 'dcp', stereoscopic: true }), {
    panel: null,
    displayProfile: null,
    stereo: null,
  });
  const mono = notApplicableReasons({ kind: 'dcp', stereoscopic: false });
  assert.equal(mono.displayProfile, null);
  assert.match(mono.stereo, /one eye/);
  assert.deepEqual(notApplicableReasons({ kind: 'nothing' }), { panel: null, displayProfile: null, stereo: null });
});
