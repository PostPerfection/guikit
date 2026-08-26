// The tauri bridge the modules talk to, standing in for @tauri-apps/api/core so
// they can be imported with no tauri and no build.

// What the page asked the backend for, in order: [command, arguments].
export const invocations = [];

// What each command answers with, for a test that reads the reply.
const answers = new Map();

export function invoke(command, args) {
  invocations.push([command, args]);
  return Promise.resolve(answers.has(command) ? answers.get(command) : '{}');
}

export function answerWith(command, value) {
  answers.set(command, value);
}

export function forgetInvocations() {
  invocations.length = 0;
}
