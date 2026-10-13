// the Ctrl chords both apps bind
export const SHARED_BINDINGS = {
  openPackage: "Ctrl+Shift+O",
  build: "Ctrl+B",
  preview: "Ctrl+P",
  importVideo: "Ctrl+I",
};

export const THEME_SHORTCUT = {
  id: "toggle-theme",
  label: "Toggle light / dark theme",
  category: "Appearance",
  binding: "Ctrl+Shift+T",
  buttonId: "theme-toggle",
};

const POST_BUILD_ROW_ID = "post-build-actions";
const FINISHED_BUILD_CATEGORY = "Finished build";

function postBuildRowShown() {
  return document.getElementById(POST_BUILD_ROW_ID)?.hidden === false;
}

export const POST_BUILD_SHORTCUTS = [
  { id: "post-build-play", label: "Play the finished build", binding: "Ctrl+Shift+P" },
  { id: "post-build-queue", label: "Add the finished build to the playlist", binding: "Ctrl+Shift+A" },
  { id: "post-build-inspect", label: "Inspect the finished build", binding: "Ctrl+Shift+E" },
  { id: "post-build-reveal", label: "Show the finished build in its folder", binding: "Ctrl+Shift+F" },
].map((shortcut) => ({ ...shortcut, category: FINISHED_BUILD_CATEGORY, buttonId: shortcut.id, when: postBuildRowShown }));

const SIDEBAR_VIEW_SELECTOR = ".sidebar-btn[data-view]";
const NUMBERED_VIEW_LIMIT = 9;

// Ctrl+1 to Ctrl+9 follow the sidebar from the top
export function sidebarViewShortcuts(switchView) {
  return [...document.querySelectorAll(SIDEBAR_VIEW_SELECTOR)].slice(0, NUMBERED_VIEW_LIMIT).map((button, index) => ({
    id: `view-${button.dataset.view}`,
    label: button.title,
    category: "Views",
    binding: `Ctrl+${index + 1}`,
    button,
    handler: () => switchView(button.dataset.view),
  }));
}
