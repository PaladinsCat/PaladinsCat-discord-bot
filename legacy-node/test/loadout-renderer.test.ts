import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { AssetCatalog } from '../src/asset-catalog.js';
import { MatchRenderer, scaleCardDescription } from '../src/match-renderer.js';
import type { LoadoutRenderRecord } from '../src/types.js';

test('scales Paladins card tokens cumulatively for the selected level', () => {
  assert.equal(
    scaleCardDescription('[Fireball] Reduce the Cooldown by {scale=0.4|0.4}s.', 4),
    'Reduce the Cooldown by 1.6s.',
  );
  assert.equal(
    scaleCardDescription('[Weapon] Increase Ammo by {1|1}.', 5),
    'Increase Ammo by 5.',
  );
  assert.equal(
    scaleCardDescription('[Shield] Increase Shield Health by {2,000|-200}.', 4),
    'Increase Shield Health by 1,400.',
  );
});

test('loadout canvas reuses scoreboard glass, dimming, typography and badge geometry', () => {
  const workspaceAssetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const assetRoot = fs.existsSync(workspaceAssetRoot) ? workspaceAssetRoot : path.resolve(process.cwd(), 'assets');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const record: LoadoutRenderRecord = {
    player: { id: '1', name: 'Player' },
    loadout: {
      id: '1', deck_id: '1', deck_key: 'deck', champion_id: 2438, champion_name: 'Strix',
      loadout_name: 'Main', card_ids: [19379, 19463, 19387, 19441, 19475], card_levels: [5, 2, 2, 3, 3],
      talent_id: null, fetched_at: '2026-07-21T00:00:00Z', updated_at: '2026-07-21T00:00:00Z',
    },
  };
  const html = (renderer as unknown as { loadoutDocument(value: LoadoutRenderRecord): string }).loadoutDocument(record);

  assert.match(html, /--bg:\s*#161618/);
  assert.match(html, /filter:saturate\(1\.15\);opacity:\.7/);
  assert.match(html, /background:rgba\(5,9,15,\.58\)/);
  assert.match(html, /backdrop-filter:blur\(7px\)/);
  assert.match(html, /height:238px/);
  assert.match(html, /font-size:14px;line-height:1\.25;font-weight:700/);
  assert.match(html, /left:13\.2%;top:92\.7%/);
  assert.match(html, /transform:translate\(-47%,-44%\)/);
  assert.doesNotMatch(html, /Card points|paladinscat\.com/);
});
