// What the page asked the window for, in order: each setFullscreen value.
export const fullscreenRequests = [];

export function getCurrentWindow() {
  return {
    async setFullscreen(fullscreen) {
      fullscreenRequests.push(fullscreen);
    },
  };
}
