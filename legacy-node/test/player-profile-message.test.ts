import assert from 'node:assert/strict';
import test from 'node:test';
import { validateDiscordMessage } from '../src/discord-message.js';
import { renderDiscordPreview } from '../src/discord-preview.js';
import { buildPlayerProfileMessage } from '../src/player-profile-message.js';

test('player profile message uses a compact, Discord-safe profile layout', () => {
  const payload = buildPlayerProfileMessage({
    player: {
      id: '42', name: 'Name_with_*markdown*', level: 999, region: 'NA', platform: 'Steam',
      title: '<font color="#ff00ff">Champion *of* Tides</font>', wins: 5506, losses: 3830,
      hours_played: 2920, kbm_tier: 26, kbm_rank: 12, kbm_points: 1234, kbm_wins: 24, kbm_losses: 14,
      controller_tier: 0, avg_dpm: 5234.6, avg_hpm: null, avg_mpm: 455.2,
      avatar_url: 'https://cdn.example/avatar.png', last_updated: '2026-07-14T00:00:00Z',
    },
    profileRefresh: { refreshed_at: '2026-07-14T00:00:00Z' },
    championRatings: [
      { champion_name: 'Androxus', mu: 1820, matches_played: 100 },
      { champion_name: 'Ash', mu: 1700, matches_played: 90 },
    ],
  }, 'https://paladinscat.com');

  assert.deepEqual(validateDiscordMessage(payload), []);
  assert.deepEqual(payload.allowedMentions, { parse: [] });
  const embed = payload.embeds?.[0];
  assert.ok(embed);
  assert.equal(embed.url, 'https://paladinscat.com/players/42');
  assert.equal(embed.thumbnail?.url, 'https://cdn.example/avatar.png');
  assert.match(embed.title ?? '', /Name\\_with/);
  assert.ok((embed.description ?? '').includes('Champion \\*of\\* Tides'));
  assert.ok(embed.fields?.some((field) => field.name === 'Ranked KBM' && field.value.includes('Grandmaster #12')));
  assert.ok(!embed.fields?.some((field) => field.name === 'Recent form'));
  assert.ok(embed.fields?.some((field) => field.name === 'Top champions'));
  const preview = renderDiscordPreview(payload);
  assert.match(preview, /Discord message preview/);
  assert.match(preview, /Mentions disabled/);
  assert.match(preview, /Exact Discord payload/);
});

test('player profile message uses the local avatar when Hi-Rez has no image link', () => {
  const payload = buildPlayerProfileMessage({
    player: { id: '42', name: 'Fallback avatar', avatar_url: null },
  }, 'https://paladinscat.com/');

  const embed = payload.embeds?.[0];
  assert.equal(embed?.thumbnail?.url, 'https://paladinscat.com/images/icons/Avatar_Default_Icon.png');
  const preview = renderDiscordPreview(payload);
  assert.match(preview, /Avatar_Default_Icon\.avif/);
  assert.match(preview, /Avatar_Default_Icon\.png/);
});

test('Discord validator rejects payloads that exceed platform limits', () => {
  const errors = validateDiscordMessage({
    content: 'x'.repeat(2001),
    embeds: [{ title: 'x'.repeat(257) }],
    allowedMentions: { parse: [] },
  });
  assert.equal(errors.length, 2);
  assert.match(errors[0] ?? '', /content/);
  assert.match(errors[1] ?? '', /title/);
});
