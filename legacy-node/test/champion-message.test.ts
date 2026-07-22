import assert from 'node:assert/strict';
import test from 'node:test';
import { buildChampionPayload } from '../src/message-builders.js';
import { validateDiscordMessage } from '../src/discord-message.js';

const result = {
  champion: { name: 'Androxus', title: 'The Godslayer', roles: 'Flank' },
  stats: {
    avg_league_tier: 17.5,
    win_rate: 50.8,
    wins: 2811,
    losses: 2722,
    total_plays: 5533,
  },
  championPerformance: {
    dpm: { championName: 'Androxus', className: 'Flank', avgValue: 5383, p10: 3291, p90: 7597 },
    wpm: { avgValue: 4360, p10: 2582, p90: 6227 },
    apm: { avgValue: 1031, p10: 416, p90: 1682 },
    gpm: { avgValue: 238, p10: 164, p90: 319 },
    hpm: { avgValue: 11, p10: 0, p90: 594 },
    mpm: { avgValue: 1, p10: 0, p90: 401 },
    kda: { avgValue: 3, p10: 0.5, p90: 6.4 },
  },
  talentStats: {
    talentCoveredMatches: 5377,
    talents: [
      { talentName: 'Dark Stalker', totalPlays: 3400, winRate: 52.5 },
      { talentName: 'Defiant Fist', totalPlays: 1000, winRate: 50.7 },
      { talentName: 'Godslayer', totalPlays: 977, winRate: 44.9 },
    ],
  },
};

test('champion payload presents database performance metrics for the selected lobby', () => {
  const payload = buildChampionPayload(result, 'https://paladinscat.example', 'Diamond+ lobbies');
  assert.deepEqual(validateDiscordMessage(payload), []);
  const embed = payload.embeds?.[0];
  assert.equal(embed?.title, 'Androxus · Ranked performance');
  assert.equal(embed?.url, 'https://paladinscat.example/champions/androxus');
  assert.match(embed?.description ?? '', /Diamond\+ lobbies/);
  assert.match(embed?.fields?.find((field) => field.name === 'Average lobby tier')?.value ?? '', /Platinum III/);
  assert.match(embed?.fields?.find((field) => field.name === 'DPM')?.value ?? '', /5,383/);
  assert.match(embed?.fields?.find((field) => field.name === 'Most played talents')?.value ?? '', /Dark Stalker/);
});

test('champion payload remains compatible with page bundles created before metadata was added', () => {
  const { champion: _champion, ...legacyResult } = result;
  const payload = buildChampionPayload(legacyResult, 'https://paladinscat.example');
  assert.equal(payload.embeds?.[0]?.title, 'Androxus · Ranked performance');
  assert.equal(payload.embeds?.[0]?.fields?.find((field) => field.name === 'Class')?.value, 'Flank');
});
