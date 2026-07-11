import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import sharp from 'sharp';
import { AssetCatalog } from '../src/asset-catalog.js';
import { MatchRenderer } from '../src/match-renderer.js';
import type { MatchRecord } from '../src/types.js';

test('renders an optimized 1280x720 JPEG from shared frontend assets', async () => {
  const players = Array.from({ length: 10 }, (_, index) => ({
    player_id: String(index + 1), player_name: `Player ${index + 1}`,
    champion_id: 2205, champion_name: 'Androxus', kills: index, deaths: 5,
    assists: 10, damage_done_physical: 65000, damage_taken: 45000,
    damage_mitigated: 0, healing: 0, gold_earned: 3000,
    win_status: index < 5 ? 'Winner' : 'Loser', task_force: index < 5 ? 1 : 2,
    league_tier: 15, source: 'direct', private_slot: 0,
  }));
  const record: MatchRecord = {
    match: { match_id: '123456789', entry_datetime: new Date().toISOString(), queue_id: 486,
      duration_seconds: 900, region: 'NA', map: 'Ranked Brightmarsh', team1_score: 4,
      team2_score: 2, winning_task_force: 1, broken: false, recovered: true, private: false },
    players,
  };
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const output = await new MatchRenderer(new AssetCatalog(assetRoot)).render(record);
  const metadata = await sharp(output).metadata();
  assert.equal(metadata.format, 'jpeg');
  assert.equal(metadata.width, 1280);
  assert.equal(metadata.height, 720);
  assert.ok(output.byteLength < 1024 * 1024);
});
