import fs from 'node:fs/promises';
import path from 'node:path';
import { loadConfig } from '../src/config.js';
import { PaladinsCatApi } from '../src/api-client.js';
import { AssetCatalog } from '../src/asset-catalog.js';
import { MatchRenderer } from '../src/match-renderer.js';

const matchId = process.argv[2];
if (!matchId) throw new Error('Usage: npm run render:sample -- <match-id> [output.jpg]');
const config = loadConfig();
const record = await new PaladinsCatApi(config.apiUrl).match(matchId);
const output = path.resolve(process.argv[3] ?? `paladinscat-match-${matchId}.jpg`);
await fs.writeFile(output, await new MatchRenderer(new AssetCatalog(config.assetRoot)).render(record));
console.log(output);
