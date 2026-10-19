// The tauri bridge the modules talk to, standing in for @tauri-apps/api/core so
// they can be imported with no tauri and no build.

// What the page asked the backend for, in order: [command, arguments].
export const invocations = [];

// What each command answers with, for a test that reads the reply.
const answers = new Map();
// What each refused command rejects with.
const refusals = new Map();
// The replies a test settles itself, one invocation each.
const deferred = new Map();

export function invoke(command, args) {
  invocations.push([command, args]);
  if (deferred.has(command)) {
    const reply = deferred.get(command);
    deferred.delete(command);
    return reply;
  }
  if (refusals.has(command)) return Promise.reject(refusals.get(command));
  return Promise.resolve(answers.has(command) ? answers.get(command) : '{}');
}

export function answerWith(command, value) {
  answers.set(command, value);
}

export function refuseWith(command, error) {
  refusals.set(command, error);
}

export function forgetRefusals() {
  refusals.clear();
}

export function forgetInvocations() {
  invocations.length = 0;
}

// the next invocation of `command` waits until the test calls resolve or reject
export function deferAnswer(command) {
  let settle;
  deferred.set(
    command,
    new Promise((resolve, reject) => {
      settle = { resolve, reject };
    }),
  );
  return settle;
}
