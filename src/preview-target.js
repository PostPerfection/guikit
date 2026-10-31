export const PREVIEW_KIND_SOURCE = "source";
export const PREVIEW_KIND_PACKAGE = "package";
export const PREVIEW_KIND_TIMELINE = "timeline";

// the picked asset or package, else the package the last build wrote, else nothing to preview
export function previewTarget({ selectedPreview, builtPackage }) {
  if (selectedPreview) return { kind: selectedPreview.kind, path: selectedPreview.path };
  if (builtPackage) return { kind: PREVIEW_KIND_PACKAGE, path: builtPackage };
  return null;
}

// with Play whole timeline ticked, a picture a timeline clip shows plays as the whole timeline from that clip's start
export function sourcePreviewTarget(path, clip, playsWholeTimeline) {
  if (clip === null || !playsWholeTimeline) return { kind: PREVIEW_KIND_SOURCE, path };
  return { kind: PREVIEW_KIND_TIMELINE, startFrame: clip.start };
}

export function previewButtonEnabled(target, shownPath, playbackEnded) {
  return target !== null && (target.path !== shownPath || playbackEnded);
}
