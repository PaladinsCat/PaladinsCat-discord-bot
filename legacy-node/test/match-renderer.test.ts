import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import sharp from 'sharp';
import { AssetCatalog } from '../src/asset-catalog.js';
import { MatchRenderer, matchPlayerDisplayTier } from '../src/match-renderer.js';
import type { MatchRecord } from '../src/types.js';

const chromiumPath = process.env.PALADINSCAT_CHROMIUM_PATH;
const requiresChromium = !chromiumPath || !fs.existsSync(chromiumPath);

test('promotes only top-100 Master players to the Grandmaster display tier', () => {
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 26, kbm_rank: 1 }), 27);
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 26, kbm_rank: 100 }), 27);
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 26, kbm_rank: 101 }), 26);
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 26 }), 26);
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 25, kbm_rank: 1 }), 25);
  assert.equal(matchPlayerDisplayTier({ kbm_tier: 26, profile_snapshot: { kbm_rank: 1 } }), 27);
});

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
  await renderer.close();
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

test("embeds Io's canonical talent artwork when the API name differs from the asset name", () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const record: MatchRecord = {
    match: { match_id: '1280893915', entry_datetime: '2026-07-20T03:03:00Z', queue_id: 424,
      duration_seconds: 508, region: 'NA', map: 'Splitstone Quarry', team1_score: 0,
      team2_score: 4, winning_task_force: 2, broken: false, recovered: true, private: false },
    players: [{
      player_id: '735721787', player_name: 'xSirris', champion_id: 2517, champion_name: 'Io',
      kills: 2, deaths: 2, assists: 8, damage_done_physical: 11341, damage_taken: 20458,
      damage_mitigated: 0, healing: 51191, gold_earned: 2850, objective_assists: 116,
      final_match_level: 326, tier: 0, win_status: 'Winner', task_force: 2,
      league_tier: 0, source: 'direct', private_slot: 0,
    }],
    facts: [{
      player_id: '735721787',
      talents: [{ talent_id: 24674, talent_name: "Goddess' Blessing", champion_name: 'Io' }],
    }],
  };

  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  const markup = html.slice(html.indexOf('</style>'));
  assert.match(markup, /class="talent-icon" src="data:image\/png;base64,[^"]+" alt="Goddess&#39; Blessing"/);
  assert.doesNotMatch(markup, /class="talent-icon" src="" alt="Goddess&#39; Blessing"/);
});

test('derives compact party groups and never renders raw singleton party IDs or empty talent images', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const partyIds = [168453, 168453, 168453, 168453, 171763, 172817, 171383, 173096, 172267, 172267];
  const players = partyIds.map((partyId, index) => ({
    player_id: String(index + 1),
    player_name: `Player ${index + 1}`,
    champion_id: 2205,
    champion_name: 'Androxus',
    kills: 1,
    deaths: 1,
    assists: 1,
    damage_done_physical: 1000,
    damage_taken: 1000,
    damage_mitigated: 0,
    healing: 0,
    gold_earned: 1000,
    party_id: partyId,
    win_status: index < 5 ? 'Winner' : 'Loser',
    task_force: index < 5 ? 1 : 2,
    league_tier: 0,
    source: 'direct',
  }));
  const record: MatchRecord = {
    match: {
      match_id: '1281027944',
      entry_datetime: '2026-07-26T08:28:00Z',
      queue_id: 424,
      duration_seconds: 1097,
      region: 'NA',
      map: 'Jaguar Falls',
      team1_score: 4,
      team2_score: 3,
      winning_task_force: 1,
      broken: false,
      recovered: false,
      private: false,
    },
    players,
    facts: [],
  };

  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  const markup = html.slice(html.indexOf('</style>'));
  assert.equal((markup.match(/title="Party 1"/g) ?? []).length, 4);
  assert.equal((markup.match(/title="Party 2"/g) ?? []).length, 2);
  assert.doesNotMatch(markup, /title="Party (?:168453|171763|172817|171383|173096|172267)"/);
  assert.equal((markup.match(/class="talent-icon talent-empty"/g) ?? []).length, 10);
  assert.doesNotMatch(markup, /class="talent-icon" src=""/);
});

test('shows the approved casual hero while preserving ranked metadata coordinates', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const record: MatchRecord = {
    match: { match_id: '1280758080', entry_datetime: '2026-07-14T00:35:00Z', queue_id: 424,
      duration_seconds: 768, region: 'NA', map: "LIVE Warder's Gate", team1_score: 2,
      team2_score: 4, winning_task_force: 2, broken: false, recovered: true, private: false },
    players: [],
    bans: [
      { ban_slot: 1, champion_id: 1, champion_name: 'Imani' },
      { ban_slot: 2, champion_id: 2, champion_name: 'Khan' },
    ],
  };
  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  assert.match(html, /\.match-meta\.casual-meta \.tier-meta \{ visibility: hidden; \}/);
  assert.doesNotMatch(html, /\.match-meta\.casual-meta \{ grid-template-areas:/);
  const markup = html.slice(html.indexOf('</style>'));
  assert.match(markup, /<header class="hero casual">/);
  assert.match(markup, /<div class="score casual">/);
  assert.match(markup, /<span class="status-tag casual">Casual<\/span>/);
  assert.match(markup, /<span class="status-tag recovered">Recovered<\/span>/);
  assert.match(markup, /<div class="match-context"><span>NA<\/span><span>Siege<\/span><\/div>/);
  assert.doesNotMatch(markup, /Queue 424/);
  assert.match(markup, /Warder&#39;s Gate/);
  assert.doesNotMatch(markup, /score-bans/);
  assert.match(markup, /<div class="match-meta casual-meta"><div class="tier-meta" aria-hidden="true">/);
  assert.match(markup, /Jul 14, 2026 · 12:35 AM UTC/);
  assert.match(markup, /class="duration-meta"/);
  assert.match(markup, /class="match-id-meta"/);
});

test('renders an unknown Siege score as a question mark', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const record: MatchRecord = {
    match: { match_id: '1280787404', entry_datetime: '2026-07-14T18:48:30Z', queue_id: 486,
      duration_seconds: 900, region: 'NA', map: "Ranked Warder's Gate", team1_score: null,
      team2_score: 4, winning_task_force: 2, broken: true, recovered: true, private: false },
    players: [],
  };

  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  const markup = html.slice(html.indexOf('</style>'));
  assert.match(markup, /team-one-score">\?<\/span>/);
  assert.match(markup, /team-two-score">4<\/span>/);
  assert.doesNotMatch(markup, /team-one-score">null<\/span>/);
});

test('renders moderation tags and the full-row police pattern for confirmed cheaters', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const basePlayer = {
    player_id: '1', player_name: 'Flagged Player', champion_id: 2205, champion_name: 'Androxus',
    kills: 10, deaths: 2, assists: 8, damage_done_physical: 65000, damage_taken: 45000,
    damage_mitigated: 0, healing: 0, gold_earned: 3000, final_match_level: 100, tier: 15,
    win_status: 'Winner', task_force: 1, league_tier: 15, source: 'direct', private_slot: 0,
  };
  const record: MatchRecord = {
    match: { match_id: '1280794399', entry_datetime: '2026-07-15T01:20:00Z', queue_id: 486,
      duration_seconds: 900, region: 'NA', map: 'Ranked Ascension Peak', team1_score: 1,
      team2_score: 0, winning_task_force: 1, broken: false, recovered: true, private: false },
    players: [
      { ...basePlayer, cheater: true },
      { ...basePlayer, player_id: '2', player_name: 'Sus Player', sus_count: 3, verified: true },
    ],
  };

  const html = (renderer as unknown as { document(value: MatchRecord): string }).document(record);
  assert.match(html, /body\{--cheater-pattern:url\("data:image\/svg\+xml,/);
  assert.match(html, /class="player-row grid-row cheater-row"/);
  assert.match(html, /class="player-status-tag cheater">CHEATER<\/span>/);
  assert.match(html, /class="player-status-tag suspicious">SUS<\/span>/);
  assert.match(html, /class="verified-player-icon" src="data:image\/png;base64,/);
  assert.match(html, /alt="Verified PaladinsCat player"/);
});

test('maps other live queues to human-readable game modes without exposing IDs', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const base: MatchRecord = {
    match: { match_id: '1', entry_datetime: new Date().toISOString(), queue_id: 452,
      duration_seconds: 600, region: 'EU', map: 'Marauder\'s Port', team1_score: 1,
      team2_score: 0, winning_task_force: 1, broken: false, recovered: true, private: false },
    players: [],
  };
  const document = (value: MatchRecord) => (renderer as unknown as { document(record: MatchRecord): string }).document(value);
  assert.match(document(base), /<div class="match-context"><span>EU<\/span><span>Onslaught<\/span><\/div>/);
  const deathmatch = { ...base, match: { ...base.match, queue_id: 469 } };
  assert.match(document(deathmatch), /<div class="match-context"><span>EU<\/span><span>Team Deathmatch<\/span><\/div>/);
  assert.doesNotMatch(document(deathmatch), /Queue 469/);
});

test('renders mutually exclusive recovery tags and the private tag', () => {
  const assetRoot = path.resolve(process.cwd(), '../frontend/public/images');
  const renderer = new MatchRenderer(new AssetCatalog(assetRoot));
  const base: MatchRecord = {
    match: { match_id: '1', entry_datetime: '2026-07-14 00:35:00', queue_id: 486,
      duration_seconds: 600, region: 'EU', map: 'Ranked Stone Keep (Classic)', team1_score: 1,
      team2_score: 0, winning_task_force: 1, broken: true, recovered: true, private: true },
    players: [],
  };
  const document = (value: MatchRecord) => (renderer as unknown as { document(record: MatchRecord): string }).document(value);
  const recovered = document(base);
  assert.match(recovered, /status-tag ranked">Ranked/);
  assert.match(recovered, /status-tag recovered">Recovered/);
  assert.match(recovered, /status-tag private">Private/);
  assert.doesNotMatch(recovered, /status-tag broken">Broken/);
  assert.match(recovered, /Jul 14, 2026 · 12:35 AM UTC/);

  const broken = document({ ...base, match: { ...base.match, recovered: false } });
  assert.match(broken, /status-tag broken">Broken/);
  assert.match(broken, /status-tag private">Private/);
  assert.doesNotMatch(broken, /status-tag recovered">Recovered/);
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
  await renderer.close();
  assert.equal(renderer.theme, 'light');
  assert.equal((await sharp(output).metadata()).format, 'png');
});
