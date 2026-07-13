import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import sharp from 'sharp';
import { AssetCatalog } from '../src/asset-catalog.js';
import { MatchRenderer } from '../src/match-renderer.js';
import type { MatchRecord } from '../src/types.js';

const chromiumPath = process.env.PALADINSCAT_CHROMIUM_PATH;
const requiresChromium = !chromiumPath || !fs.existsSync(chromiumPath);

test('renders the dark-default lossless 2048x1152 PNG from shared frontend assets', { skip: requiresChromium && 'requires PALADINSCAT_CHROMIUM_PATH for the CSS-native browser renderer' }, async () => {
  const players = Array.from({ length: 10 }, (_, index) => ({
    player_id: String(index + 1), player_name: `Player ${index + 1}`,
    champion_id: 2205, champion_name: 'Androxus', kills: index, deaths: 5,
    assists: 10, damage_done_physical: 65000, damage_taken: 45000,
    damage_mitigated: 0, healing: 0, gold_earned: 3000,
    final_match_level: 100 + index, tier: 15,
    win_status: index < 5 ? 'Winner' : 'Loser', task_force: index < 5 ? 1 : 2,
    league_tier: 15, source: 'direct', private_slot: 0,
  }));
  const record: MatchRecord = {
    match: { match_id: '123456789', entry_datetime: new Date().toISOString(), queue_id: 486,
      duration_seconds: 900, region: 'NA', map: 'Ranked Brightmarsh', team1_score: 4,
      team2_score: 2, winning_task_force: 1, broken: false, recovered: true, private: false },
    players,
    facts: players.map((player) => ({
      player_id: player.player_id,
      talents: [{ talent_id: 1, talent_name: 'Godslayer', champion_name: 'Androxus' }],
    })),
  };
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  assert.equal(renderer.theme, 'dark');
  const output = await renderer.render(record);
  const metadata = await sharp(output).metadata();
  assert.equal(metadata.format, 'png');
  assert.equal(metadata.width, 2048);
  assert.equal(metadata.height, 1152);
  assert.ok(output.byteLength < 4 * 1024 * 1024);
});

test('resolves WIP map names to the shared ranked map art', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const assets = new AssetCatalog(assetRoot);
  assert.match(assets.mapImage('WIP Serpent Beach V2') ?? '', /Ranked_Serpent_Beach/i);
});

test('hides ranked-only bans and average tier for casual match images', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const record: MatchRecord = {
    match: { match_id: '1280758080', entry_datetime: new Date().toISOString(), queue_id: 424,
      duration_seconds: 768, region: 'NA', map: "LIVE Warder's Gate", team1_score: 2,
      team2_score: 4, winning_task_force: 2, broken: false, recovered: true, private: false },
    players: [],
    bans: [
      { ban_slot: 1, champion_id: 1, champion_name: 'Imani' },
      { ban_slot: 2, champion_id: 2, champion_name: 'Khan' },
    ],
  };
  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  const markup = html.slice(html.indexOf('</style>'));
  assert.match(markup, /<header class="hero casual">/);
  assert.match(markup, /<div class="score casual">/);
  assert.match(markup, /<span>NA<\/span><span>Casual<\/span><span>Queue 424<\/span>/);
  assert.match(markup, /Warder&#39;s Gate/);
  assert.doesNotMatch(markup, /score-bans/);
  assert.doesNotMatch(markup, /tier-meta/);
  assert.doesNotMatch(markup, /Avg tier/i);
});

test('keeps the prototype light theme available to renderer consumers', { skip: requiresChromium && 'requires PALADINSCAT_CHROMIUM_PATH for the CSS-native browser renderer' }, async () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot), { theme: 'light' });
  const record: MatchRecord = {
    match: { match_id: '123456789', entry_datetime: new Date().toISOString(), queue_id: 486,
      duration_seconds: 900, region: 'NA', map: 'Ranked Brightmarsh', team1_score: 4,
      team2_score: 2, winning_task_force: 1, broken: false, recovered: true, private: false },
    players: Array.from({ length: 10 }, (_, index) => ({
      player_id: String(index + 1), player_name: `Player ${index + 1}`,
      champion_id: 2205, champion_name: 'Androxus', kills: index, deaths: 5,
      assists: 10, damage_done_physical: 65000, damage_taken: 45000,
      damage_mitigated: 0, healing: 0, gold_earned: 3000,
      final_match_level: 100 + index, tier: 15,
      win_status: index < 5 ? 'Winner' : 'Loser', task_force: index < 5 ? 1 : 2,
      league_tier: 15, source: 'direct', private_slot: 0,
    })),
  };
  const output = await renderer.render(record);
  assert.equal(renderer.theme, 'light');
  assert.equal((await sharp(output).metadata()).format, 'png');
});
