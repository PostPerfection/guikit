// The titles of the builds this window has queued or running, so Build greys
// only for a title that is already building.
const titlesInFlight = new Set();

export function beginBuild(title) {
  titlesInFlight.add(title);
}

export function endBuild(title) {
  titlesInFlight.delete(title);
}

export function buildInFlight(title) {
  return titlesInFlight.has(title);
}

export function anyBuildInFlight() {
  return titlesInFlight.size > 0;
}
