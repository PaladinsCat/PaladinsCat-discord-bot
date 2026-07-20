import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { AssetCatalog } from '../src/asset-catalog.js';

type TalentReference = {
  id: number;
  name: string;
  championName: string;
  iconUrl: string;
};

const workspaceAssetRoot = path.resolve(process.cwd(), '../frontend/public/images');
const assetRoot = fs.existsSync(workspaceAssetRoot)
  ? workspaceAssetRoot
  : path.resolve(process.cwd(), 'assets');
const referencePath = path.resolve(assetRoot, '../data/paladins-talent-reference.json');
const references = JSON.parse(fs.readFileSync(referencePath, 'utf8')) as TalentReference[];

test('resolves Io and Grover historical talent names through canonical talent IDs', () => {
  const assets = new AssetCatalog(assetRoot);
  assert.match(
    assets.talentIcon(24674, 'Io', "Goddess' Blessing") ?? '',
    /Talent Io Goddess's Blessing\.png$/,
  );
  assert.match(
    assets.talentIcon(20249, 'Grover', 'Great Oak') ?? '',
    /Talent Grover Wisps of Sylvanus\.png$/,
  );
});

test('resolves every observed talent to its canonical local artwork', () => {
  const assets = new AssetCatalog(assetRoot);
  const observed = references.filter((talent) => talent.championName && talent.iconUrl.startsWith('/images/champions/'));
  assert.ok(observed.length > 150, 'canonical talent reference unexpectedly lost observed talents');

  const unresolved: string[] = [];
  const mismatched: string[] = [];
  for (const talent of observed) {
    const resolved = assets.talentIcon(talent.id, talent.championName, talent.name);
    if (!resolved) {
      unresolved.push(`${talent.id} ${talent.championName} — ${talent.name}`);
      continue;
    }
    const expectedStem = path.parse(talent.iconUrl).name;
    if (path.parse(resolved).name !== expectedStem) {
      mismatched.push(`${talent.id} expected ${expectedStem}, received ${path.parse(resolved).name}`);
    }
  }

  assert.deepEqual(unresolved, []);
  assert.deepEqual(mismatched, []);
});
