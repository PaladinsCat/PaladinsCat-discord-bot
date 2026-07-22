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
      { player_id: '42', player_name: 'Point_Tank', champion_name: 'Ash', kbm_tier: 26, profile_win_rate: 54.8, queue_elo: 1842.4, task_force: 1 },
      { player_id: '43', player_name: 'Support', champion_name: 'Furia', kbm_tier: 15, profile_win_rate: 51.2, queue_elo: 1518.7, task_force: 1 },
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
  assert.match(first, /Global 54\.8% WR.*1,842 ELO/);
  assert.match(first, /https:\/\/paladinscat\.com\/players\/42/);
  assert.doesNotMatch(first, /localhost/);
  assert.match(first, /\*\*Furia\*\*.*Gold I/);
  assert.match(first, /Global 51\.2% WR.*1,519 ELO/);
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

test('current match estimates complementary team win chances from database stats', () => {
  const payload = buildCurrentPayload({
    player_id: '1',
    match: { match_id: '9002', queue_id: 486, map: 'Bazaar', region: 'EU' },
    players: [
      { player_id: '1', player_name: 'One', champion_name: 'Ash', task_force: 1, queue_elo: 1800, profile_win_rate: 55 },
      { player_id: '2', player_name: 'Two', champion_name: 'Furia', task_force: 1, queue_elo: 1700, profile_win_rate: 52 },
      { player_id: '3', player_name: 'Three', champion_name: 'Tyra', task_force: 1, queue_elo: 1600, profile_win_rate: 50 },
      { player_id: '4', player_name: 'Four', champion_name: 'Barik', task_force: 2, queue_elo: 1600, profile_win_rate: 50 },
      { player_id: '5', player_name: 'Five', champion_name: 'Vatu', task_force: 2, queue_elo: 1500, profile_win_rate: 48 },
      { player_id: '6', player_name: 'Six', champion_name: 'Ying', task_force: 2, queue_elo: 1400, profile_win_rate: 45 },
    ],
  }, 'https://paladinscat.com');

  assert.deepEqual(validateDiscordMessage(payload), []);
  assert.equal(payload.embeds?.[0]?.fields?.[0]?.name, 'Team 1 · 72% win chance');
  assert.equal(payload.embeds?.[0]?.fields?.[1]?.name, 'Team 2 · 28% win chance');
  assert.match(payload.embeds?.[0]?.footer?.text ?? '', /Estimate blends queue ELO with global win rate/);
});

test('current match omits an estimate when either team lacks enough ELO coverage', () => {
  const payload = buildCurrentPayload({
    player_id: '1',
    match: { match_id: '9003', queue_id: 486, map: 'Bazaar', region: 'EU' },
    players: [
      { player_id: '1', player_name: 'One', champion_name: 'Ash', task_force: 1, queue_elo: 1700 },
      { player_id: '2', player_name: 'Two', champion_name: 'Furia', task_force: 1 },
      { player_id: '3', player_name: 'Three', champion_name: 'Tyra', task_force: 1 },
      { player_id: '4', player_name: 'Four', champion_name: 'Barik', task_force: 2, queue_elo: 1600 },
      { player_id: '5', player_name: 'Five', champion_name: 'Vatu', task_force: 2 },
      { player_id: '6', player_name: 'Six', champion_name: 'Ying', task_force: 2 },
    ],
  }, 'https://paladinscat.com');

  assert.equal(payload.embeds?.[0]?.fields?.[0]?.name, 'Team 1');
  assert.equal(payload.embeds?.[0]?.fields?.[1]?.name, 'Team 2');
  assert.doesNotMatch(payload.embeds?.[0]?.footer?.text ?? '', /Estimate/);
});
