const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;
const TIME_FIELD_DIGITS = 2;

function twoDigits(value) {
  return String(value).padStart(TIME_FIELD_DIGITS, '0');
}

/// A frame count at `numerator / denominator` frames a second as HH:MM:SS, the
/// part second dropped.
export function framesToTimecode(frames, [numerator, denominator]) {
  const totalSeconds = Math.floor((frames * denominator) / numerator);
  const hours = Math.floor(totalSeconds / SECONDS_PER_HOUR);
  const minutes = Math.floor((totalSeconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  return `${twoDigits(hours)}:${twoDigits(minutes)}:${twoDigits(seconds)}`;
}

/// An RFC 3339 timestamp as `YYYY-MM-DD HH:MM` in local time.
export function formatDateTime(rfc3339) {
  const date = new Date(rfc3339);
  const day = `${date.getFullYear()}-${twoDigits(date.getMonth() + 1)}-${twoDigits(date.getDate())}`;
  return `${day} ${twoDigits(date.getHours())}:${twoDigits(date.getMinutes())}`;
}
