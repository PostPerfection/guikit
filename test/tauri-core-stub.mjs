// The tauri bridge preview.js talks to, standing in for @tauri-apps/api/core so
// the module can be imported with no tauri and no build.

// What the page asked the backend for, in order: [command, arguments].
export const invocations = [];

export function invoke(command, args) {
  invocations.push([command, args]);
  return Promise.resolve('{}');
}

export function forgetInvocations() {
  invocations.length = 0;
}
