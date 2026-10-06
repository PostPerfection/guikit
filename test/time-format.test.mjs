import assert from 'node:assert/strict';
import test from 'node:test';

const { framesToTimecode, formatDateTime } = await import('../src/time-format.js');

test('a 24 frame count drops the part second', () => {
  assert.equal(framesToTimecode(295368 + 23, [24, 1]), '03:25:07');
});

test('a 25 frame hour reads one hour', () => {
  assert.equal(framesToTimecode(90000, [25, 1]), '01:00:00');
});

test('a 30000/1001 count runs slower than 30 a second', () => {
  assert.equal(framesToTimecode(107892, [30000, 1001]), '00:59:59');
  assert.equal(framesToTimecode(108000, [30000, 1001]), '01:00:03');
});

test('an RFC 3339 string shows its local date and time without seconds', () => {
  process.env.TZ = 'UTC';
  assert.equal(formatDateTime('2026-10-06T14:30:45+02:00'), '2026-10-06 12:30');
  process.env.TZ = 'Asia/Tokyo';
  assert.equal(formatDateTime('2026-10-06T20:05:59Z'), '2026-10-07 05:05');
});
