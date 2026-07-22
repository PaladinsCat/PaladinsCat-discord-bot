import assert from 'node:assert/strict';
import test from 'node:test';
import { validateDiscordMessage } from '../src/discord-message.js';
import { buildCompositionPayload, buildItemsPayload, buildMapsPayload } from '../src/message-builders.js';

test('maps payload shows every returned map with play share and duration', () => {
  const payload = buildMapsPayload([
    { map: 'Stone Keep', total_matches: 1200, distribution_rate: 12.4, avg_duration_seconds: 754 },
    { map: 'Ascension Peak', total_matches: 980, distribution_rate: 10.1, avg_duration_seconds: 681 },
  ], 'https://paladinscat.example');
  assert.deepEqual(validateDiscordMessage(payload), []);
  const description = payload.embeds?.[0]?.description ?? '';
  assert.match(description, /Stone Keep/);
  assert.match(description, /1,200 matches/);
  assert.match(description, /12\.4% of pool/);
  assert.match(description, /12m 34s avg/);
  assert.match(description, /Ascension Peak/);
});

test('composition payload is limited to the five most-played rows', () => {
  const rows = Array.from({ length: 7 }, (_, index) => ({
    frontline: 1,
    damage: 2,
    flank: 1,
    support: 1,
    count: 1000 - index,
    winrate: 50 + index / 10,
  }));
  const payload = buildCompositionPayload(rows, 'https://paladinscat.example');
  assert.deepEqual(validateDiscordMessage(payload), []);
  const fields = payload.embeds?.[0]?.fields ?? [];
  assert.equal(fields.length, 5);
  assert.match(fields[0]?.name ?? '', /1\. 1 Frontline/);
  assert.match(fields[0]?.value ?? '', /1,000 matches · 50\.0% win rate/);
  assert.match(fields[4]?.name ?? '', /5\. 1 Frontline/);
  assert.equal(fields.some((field) => /^6\./.test(field.name)), false);
});

test('items payload identifies the selected lobby and displays database aggregates', () => {
  const payload = buildItemsPayload([
    { item_id: 2024, item_name: 'Chronos', total_uses: 12500, pick_rate: 18.75, win_rate: 52.34 },
  ], 'https://paladinscat.example', 'Diamond+ lobbies');
  assert.deepEqual(validateDiscordMessage(payload), []);
  const embed = payload.embeds?.[0];
  assert.match(embed?.description ?? '', /Diamond\+ lobbies/);
  assert.match(embed?.description ?? '', /Chronos/);
  assert.match(embed?.description ?? '', /18\.8% pick/);
  assert.match(embed?.description ?? '', /52\.3% WR/);
  assert.match(embed?.description ?? '', /12,500 uses/);
  assert.match(embed?.description ?? '', /https:\/\/paladinscat\.example\/game\/items\/2024/);
});
