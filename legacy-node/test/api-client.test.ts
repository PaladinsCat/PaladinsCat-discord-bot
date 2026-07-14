import assert from 'node:assert/strict';
import test from 'node:test';
import { PaladinsCatApi } from '../src/api-client.js';

function recordingFetch(urls: string[]): typeof fetch {
  return (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    const body = url.includes('/matches/123')
      ? { matches: [{ match: { match_id: '123' }, players: [] }] }
      : url.includes('/players/123?')
        ? { player: { id: '123', name: 'Database Player' } }
        : url.includes('/stats/ranked-leaderboard')
          ? []
          : {};
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
}

test('local-only bot reads suppress backend Hi-Rez fallbacks', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.player('123');
  await api.playerHistory('123', 10);
  await api.playerLoadouts('123');
  await api.match('123');

  assert.deepEqual(urls, [
    'http://backend:3005/players/123?include=ratings,champions&refresh=false',
    'http://backend:3005/players/123/matches?limit=10&refresh=false',
    'http://backend:3005/players/123/loadouts?refresh=false',
    'http://backend:3005/matches/123?refresh=false',
    'http://backend:3005/matches/fact/123',
  ]);
});

test('leaderboard supplies the required database tier', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.rankedLeaderboard(10);

  assert.deepEqual(urls, [
    'http://backend:3005/stats/ranked-leaderboard?tier=26&top=10',
  ]);
});

test('normal mode preserves database-first backend fallback behavior', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: false,
    fetchImpl: recordingFetch(urls),
  });

  await api.player('123');
  await api.match('123');

  assert.deepEqual(urls, [
    'http://backend:3005/players/123?include=ratings,champions',
    'http://backend:3005/matches/123',
    'http://backend:3005/matches/fact/123',
  ]);
});

test('match rendering hydrates profile display fields and talent facts', async () => {
  const urls: string[] = [];
  const fetchImpl = (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    const body = url.includes('/matches/fact/123')
      ? { players: [{ player_id: '1', talents: [{ talent_id: 99, talent_name: 'Godslayer', champion_name: 'Androxus' }] }] }
      : url.includes('/matches/123')
        ? { matches: [{ match: { match_id: '123', queue_id: 486 }, players: [{ player_id: '1', final_match_level: 999, account_level: 999, league_tier: 0 }] }] }
        : { player: { id: '1', name: 'Player', level: 1158, kbm_tier: 13, kbm_rank: 2 }, queueRatings: [{ queue_id: 486, mu: 1600 }] };
    return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, { localOnly: true, fetchImpl });

  const record = await api.match('123');

  assert.equal(record.players[0]?.final_match_level, 1158);
  assert.equal(record.players[0]?.tier, 13);
  assert.equal(record.players[0]?.queue_elo, 1600);
  assert.equal(record.facts?.[0]?.talents[0]?.talent_name, 'Godslayer');
  assert.deepEqual(urls, [
    'http://backend:3005/matches/123?refresh=false',
    'http://backend:3005/matches/fact/123',
    'http://backend:3005/players/1?include=ratings&refresh=false',
  ]);
});
