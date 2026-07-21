import assert from 'node:assert/strict';
import test from 'node:test';
import { validateDiscordMessage } from '../src/discord-message.js';
import { buildCurrentPayload } from '../src/message-builders.js';

test('current match renders a compact two-team lobby instead of JSON', () => {
  const payload = buildCurrentPayload({
    player_id: '42',
    match: {
      match_id: '9001', queue_id: 486, map: 'Stone Keep', region: 'NA',
      source_player_id: '42', detected_at: '2026-07-21T22:00:00Z',
    },
    players: [
      { player_id: '42', player_name: 'Point_Tank', champion_name: 'Ash', kbm_tier: 26, task_force: 1 },
      { player_id: '43', player_name: 'Support', champion_name: 'Furia', kbm_tier: 15, task_force: 1 },
      { player_id: '44', player_name: 'Flank', champion_name: 'Vatu', live_tier: 21, task_force: 2 },
      { player_id: '-1', player_name: 'Private Account', champion_name: 'Io', task_force: 2 },
    ],
  }, 'https://paladinscat.com');

  assert.deepEqual(validateDiscordMessage(payload), []);
  const embed = payload.embeds?.[0];
  assert.equal(embed?.title, 'Stone Keep · Live match');
  assert.match(embed?.description ?? '', /Ranked Siege.*NA/);
  assert.doesNotMatch(embed?.description ?? '', /```json/);
  const first = embed?.fields?.find((field) => field.name === 'Team 1')?.value ?? '';
  const second = embed?.fields?.find((field) => field.name === 'Team 2')?.value ?? '';
  assert.match(first, /▸ \*\*Ash\*\*.*Point\\_Tank.*Master/);
  assert.match(first, /\*\*Furia\*\*.*Gold I/);
  assert.match(second, /\*\*Vatu\*\*.*Diamond V/);
  assert.match(second, /Private Account/);
  assert.equal(embed?.timestamp, '2026-07-21T22:00:00.000Z');
});

test('current match uses dedicated pending and not-live states', () => {
  const pending = buildCurrentPayload({ match: null, players: [], pending: true }, 'https://paladinscat.com');
  assert.equal(pending.embeds?.[0]?.title, 'Live lobby loading');
  assert.match(pending.embeds?.[0]?.description ?? '', /Try `\/current` again shortly/);

  const offline = buildCurrentPayload({ match: null, players: [], player_id: '42' }, 'https://paladinscat.com');
  assert.equal(offline.embeds?.[0]?.title, 'Not in a live match');
  assert.doesNotMatch(offline.embeds?.[0]?.description ?? '', /json/i);
});
