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
        : url.includes('/champions/Androxus/page-data')
          ? { champion: { name: 'Androxus' }, stats: {} }
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

test('champion page data forwards the selected lobby tier range', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.championPageData('Androxus', {
    value: 'diamond',
    label: 'Diamond+ lobbies',
    tierMin: 21,
    tierMax: 26,
  });

  assert.deepEqual(urls, [
    'http://backend:3005/champions/Androxus/page-data?tierMin=21&tierMax=26',
  ]);
});

test('champion page data uses global ranked metrics when no bounds are set', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.championPageData('Androxus', { value: 'global', label: 'Global ranked lobbies' });

  assert.deepEqual(urls, [
    'http://backend:3005/champions/Androxus/page-data',
  ]);
});

test('map, composition, and item commands use ranked database aggregate routes', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.rankedMaps(100);
  await api.rankedCompositions(5);
  await api.rankedItems({ value: 'diamond', label: 'Diamond+ lobbies', tierMin: 21, tierMax: 26 }, 20);

  assert.deepEqual(urls, [
    'http://backend:3005/stats/maps?queueId=486&limit=100',
    'http://backend:3005/matches/compositions?sortBy=count&order=desc&limit=5',
    'http://backend:3005/stats/items?mode=ranked&limit=20&tierMin=21&tierMax=26',
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

test('Discord saved-player reads and writes use the service mapping contract', async () => {
  const requests: Array<{ url: string; method: string; body: unknown }> = [];
  const fetchImpl = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? 'GET';
    const body = init?.body ? JSON.parse(String(init.body)) : null;
    requests.push({ url, method, body });
    const payload = url.includes('/players/discord?')
      ? { player: { id: '716515038', name: 'NabiCookTV' } }
      : { player: { id: '716515038', name: 'NabiCookTV' } };
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl,
  });

  const saved = await api.saveDiscordPlayer('test-user-1', 'NabiCookTV');
  const read = await api.savedDiscordPlayer('test-user-1');

  assert.deepEqual(saved, { id: '716515038', name: 'NabiCookTV' });
  assert.deepEqual(read, saved);
  assert.deepEqual(requests, [
    {
      url: 'http://backend:3005/players/discord?player=NabiCookTV',
      method: 'GET',
      body: null,
    },
    {
      url: 'http://backend:3005/players/discord/saved-player',
      method: 'PUT',
      body: {
        discordUserId: 'test-user-1',
        playerId: '716515038',
      },
    },
    {
      url: 'http://backend:3005/players/discord/saved-player?discordUserId=test-user-1',
      method: 'GET',
      body: null,
    },
  ]);
});

test('legacy client never sends the retired static service credential', async () => {
  let observedToken = '';
  let observedAuthorization = '';
  const fetchImpl = (async (_input: string | URL | Request, init?: RequestInit) => {
    observedToken = new Headers(init?.headers).get('x-paladinscat-service-token') || '';
    observedAuthorization = new Headers(init?.headers).get('authorization') || '';
    return new Response(JSON.stringify({ player: { id: '123' } }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl,
  });

  await api.playerById('123');

  assert.equal(observedToken, '');
  assert.equal(observedAuthorization, '');
});

test('current match uses the enriched live-lobby projection', async () => {
  const urls: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, {
    localOnly: true,
    fetchImpl: recordingFetch(urls),
  });

  await api.liveMatch('123');

  assert.deepEqual(urls, [
    'http://backend:3005/live/players/123',
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
  assert.equal(record.players[0]?.kbm_rank, 2);
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

test('loadout refresh uses the explicit backend POST without a refresh query parameter', async () => {
  const requests: Array<{ url: string; method: string }> = [];
  const fetchImpl = (async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ url: String(input), method: String(init?.method ?? 'GET') });
    return new Response(JSON.stringify({ loadouts: [], freshness: {}, refreshed: true }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, { localOnly: true, fetchImpl });

  await api.refreshPlayerLoadoutsById('123');

  assert.deepEqual(requests, [{ url: 'http://backend:3005/players/123/loadouts/refresh', method: 'POST' }]);
});

test('API errors preserve the structured backend cooldown message', async () => {
  const fetchImpl = (async () => new Response(JSON.stringify({
    error: { code: 'LOADOUT_REFRESH_COOLDOWN', message: 'Refresh available in 42 seconds.', details: { remaining_seconds: 42 } },
  }), { status: 429, headers: { 'Content-Type': 'application/json' } })) as typeof fetch;
  const api = new PaladinsCatApi('http://backend:3005', 1000, { fetchImpl });

  await assert.rejects(
    () => api.refreshPlayerLoadoutsById('123'),
    (error: unknown) => error instanceof Error
      && error.message === 'Refresh available in 42 seconds.'
      && (error as { code?: string }).code === 'LOADOUT_REFRESH_COOLDOWN',
  );
});
