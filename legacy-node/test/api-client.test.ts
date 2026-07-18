import assert from 'node:assert/strict';
import test from 'node:test';
import { PaladinsCatApi } from '../src/api-client.js';

function recordingFetch(urls: string[]): typeof fetch {
  return (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    const body = url.includes('/matches/batch?ids=123') || url.includes('/matches/123')
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

test('database-first bot reads keep an existing match on the fast local path', async () => {
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
    'http://backend:3005/players/123?include=ratings&refresh=false',
    'http://backend:3005/players/123/matches?limit=10&refresh=false',
    'http://backend:3005/players/123/loadouts?refresh=false',
    'http://backend:3005/matches/batch?ids=123',
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
    'http://backend:3005/players/123?include=ratings',
    'http://backend:3005/matches/123',
    'http://backend:3005/matches/fact/123',
  ]);
});

test('a database miss enters durable requested-match ingestion and retries facts after persistence', async () => {
  const urls: string[] = [];
  let factReads = 0;
  const fetchImpl = (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    if (url.includes('/matches/batch?ids=456')) {
      return new Response(JSON.stringify({ matches: [], count: 0, notFound: [456] }), { status: 200 });
    }
    if (url.includes('/matches/fact/456')) {
      factReads += 1;
      if (factReads === 1) return new Response(JSON.stringify({ error: 'Match not found' }), { status: 404 });
      return new Response(JSON.stringify({ players: [{ player_id: '1', talents: [{ talent_id: 9, talent_name: 'Persisted Talent' }] }] }), { status: 200 });
    }
    if (url.endsWith('/matches/456')) {
      return new Response(JSON.stringify({
        matches: [{
          match: { match_id: '456', queue_id: 424 },
          players: [{ player_id: '1', profile_snapshot: { level: 42 } }],
        }],
      }), { status: 200 });
    }
    return new Response(JSON.stringify({ error: 'unexpected request' }), { status: 500 });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    matchTimeoutMs: 125000,
    fetchImpl,
  });

  const record = await api.match('456');

  assert.equal(record.match.match_id, '456');
  assert.equal(record.players[0]?.final_match_level, 42);
  assert.equal(record.facts?.[0]?.talents[0]?.talent_name, 'Persisted Talent');
  assert.deepEqual(urls, [
    'http://backend:3005/matches/batch?ids=456',
    'http://backend:3005/matches/fact/456',
    'http://backend:3005/matches/456',
    'http://backend:3005/matches/fact/456',
  ]);
});

test('Discord player reads use the dedicated five-minute refresh path', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.discordPlayer('New Player', true);

  assert.deepEqual(urls, [
    'http://backend:3005/players/discord?player=New+Player&history=true',
  ]);
});

test('match rendering hydrates profile display fields from the joined snapshot without N+1 profile reads', async () => {
  const urls: string[] = [];
  const fetchImpl = (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    const body = url.includes('/matches/fact/123')
      ? { players: [{ player_id: '1', talents: [{ talent_id: 99, talent_name: 'Godslayer', champion_name: 'Androxus' }] }] }
      : url.includes('/matches/batch?ids=123')
        ? { matches: [{ match: { match_id: '123', queue_id: 486 }, players: [{ player_id: '1', final_match_level: 999, account_level: 999, league_tier: 0, profile_snapshot: { level: 1158, kbm_tier: 13, kbm_rank: 2, queue_elo: 1600, cheater: true, sus_count: 4, verified: true } }] }] }
        : {};
    return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, { localOnly: true, fetchImpl });

  const record = await api.match('123');

  assert.equal(record.players[0]?.final_match_level, 1158);
  assert.equal(record.players[0]?.tier, 13);
  assert.equal(record.players[0]?.queue_elo, 1600);
  assert.equal(record.players[0]?.cheater, true);
  assert.equal(record.players[0]?.sus_count, 4);
  assert.equal(record.players[0]?.verified, true);
  assert.equal(record.facts?.[0]?.talents[0]?.talent_name, 'Godslayer');
  assert.deepEqual(urls, [
    'http://backend:3005/matches/batch?ids=123',
    'http://backend:3005/matches/fact/123',
  ]);
});

test('match verification is read from the authoritative profile snapshot', async () => {
  const fetchImpl = (async (input: string | URL | Request) => {
    const url = String(input);
    if (url.includes('/matches/fact/123')) {
      return new Response(JSON.stringify({ players: [] }), { status: 200 });
    }
    if (url.includes('/matches/batch?ids=123')) {
      return new Response(JSON.stringify({
        matches: [{
          match: { match_id: '123', queue_id: 486 },
          players: [{ player_id: '1', profile_snapshot: { verified: true } }],
        }],
      }), { status: 200 });
    }
    return new Response(JSON.stringify({ error: 'unavailable' }), { status: 503 });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, { localOnly: true, fetchImpl });

  const record = await api.match('123');

  assert.equal(record.players[0]?.verified, true);
});
