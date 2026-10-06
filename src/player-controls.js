import { withSavedChoice } from './saved-choices.js';

export const DISPLAY_PROFILE_COMMAND = 'preview_set_display_profile';
const BUILT_IN_PROFILE_TEXT = 'Built-in sRGB';

// the Rust defaults of the player controls, for a preferences file written before them
const DEFAULT_PLAYER_CONTROLS = {
  playerPicture: {
    brightness: 1,
    masksPercent: { top: 0, bottom: 0, left: 0, right: 0 },
    scaling: 'fit',
  },
  playerSound: { device: null, layout: 'stereo', delayMilliseconds: 0 },
  playerSubtitles: { offsetPercent: 0, colour: null },
  playerDisplayProfile: null,
  playerStereo: 'leftEye',
};
const PLAYER_CONTROL_KEYS = Object.keys(DEFAULT_PLAYER_CONTROLS);

const MPV_FILE_REASON = 'Not applicable: this file plays on mpv, which these controls do not reach';
const IMF_PROFILE_REASON = 'Not applicable to IMF, the picture is shown in the colour its track signals';
const IMF_STEREO_REASON = 'Not applicable to IMF, the picture plays as one eye';
const MONO_PICTURE_REASON = 'Not applicable, the picture has one eye';

// what the player does not apply to the loaded picture, keyed by preview_loaded_picture's kind
const NOT_APPLICABLE_BY_KIND = {
  nothing: { panel: null, displayProfile: null, stereo: null },
  mpvFile: { panel: MPV_FILE_REASON, displayProfile: null, stereo: null },
  dcp: { panel: null, displayProfile: null, stereo: MONO_PICTURE_REASON },
  imf: { panel: null, displayProfile: IMF_PROFILE_REASON, stereo: IMF_STEREO_REASON },
};

// the controls an app keeps in its preferences, with the defaults for any it has not stored
export function playerControlsFromPreferences(preferences) {
  return Object.fromEntries(
    PLAYER_CONTROL_KEYS.map((key) => [key, preferences[key] ?? DEFAULT_PLAYER_CONTROLS[key]]),
  );
}

export function playerControlsFromFields({
  brightness,
  maskTop,
  maskBottom,
  maskLeft,
  maskRight,
  scaling,
  soundDevice,
  soundLayout,
  soundDelayMilliseconds,
  subtitleOffsetPercent,
  subtitleColourOverridden,
  subtitleColour,
  displayProfile,
  stereo,
}) {
  return {
    playerPicture: {
      brightness: Number(brightness),
      masksPercent: {
        top: Number(maskTop),
        bottom: Number(maskBottom),
        left: Number(maskLeft),
        right: Number(maskRight),
      },
      scaling,
    },
    playerSound: {
      device: soundDevice || null,
      layout: soundLayout,
      delayMilliseconds: Math.round(Number(soundDelayMilliseconds)),
    },
    playerSubtitles: {
      offsetPercent: Number(subtitleOffsetPercent),
      colour: subtitleColourOverridden ? subtitleColour : null,
    },
    playerDisplayProfile: displayProfile || null,
    playerStereo: stereo,
  };
}

function sameValue(first, second) {
  return JSON.stringify(first) === JSON.stringify(second);
}

// the commands that bring the player from the applied controls to these, all of them when none are applied
export function playerControlCommands(applied, settings) {
  const changed = (read) => !applied || !sameValue(read(applied), read(settings));
  const commands = [];
  if (changed((controls) => controls.playerPicture)) {
    commands.push(['preview_set_picture', { picture: settings.playerPicture }]);
  }
  if (changed((controls) => controls.playerSound.device)) {
    commands.push(['preview_set_sound_device', { device: settings.playerSound.device }]);
  }
  if (changed((controls) => controls.playerSound.layout)) {
    commands.push(['preview_set_sound_layout', { layout: settings.playerSound.layout }]);
  }
  if (changed((controls) => controls.playerSound.delayMilliseconds)) {
    commands.push(['preview_set_sound_delay', { milliseconds: settings.playerSound.delayMilliseconds }]);
  }
  if (changed((controls) => controls.playerSubtitles)) {
    commands.push(['preview_set_subtitle_presentation', { subtitles: settings.playerSubtitles }]);
  }
  if (changed((controls) => controls.playerStereo)) {
    commands.push(['preview_set_stereo_output', { output: settings.playerStereo }]);
  }
  if (changed((controls) => controls.playerDisplayProfile)) {
    commands.push([DISPLAY_PROFILE_COMMAND, { profile: settings.playerDisplayProfile }]);
  }
  return commands;
}

export function soundDeviceChoices(deviceNames, savedDevice) {
  return withSavedChoice(
    deviceNames.map((name) => ({ name, label: name })),
    savedDevice,
  );
}

// the file name of the chosen profile, with either separator
export function displayProfileText(profile) {
  return profile ? profile.split(/[/\\]/).pop() : BUILT_IN_PROFILE_TEXT;
}

// why each part of the panel does nothing to the loaded picture, null where it applies
export function notApplicableReasons(loadedPicture) {
  const reasons = NOT_APPLICABLE_BY_KIND[loadedPicture.kind];
  if (loadedPicture.stereoscopic) return { ...reasons, stereo: null };
  return reasons;
}
