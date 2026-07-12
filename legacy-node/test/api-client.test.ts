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
  ]);
});
