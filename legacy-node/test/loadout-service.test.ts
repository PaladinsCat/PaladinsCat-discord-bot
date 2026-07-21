import assert from 'node:assert/strict';
import test from 'node:test';
import { PaladinsCatApi, PaladinsCatApiError } from '../src/api-client.js';
import { findPlayerChampionLoadouts } from '../src/loadout-service.js';
import type { PlayerLoadout, PlayerLoadoutsResponse } from '../src/types.js';

const freshness = {
  ttl_seconds: 86400,
  refreshed_at: '2026-07-21T00:00:00Z',
  expires_at: '2026-07-22T00:00:00Z',
  remaining_seconds: 3600,
  expired: false,
  manual_refresh_available_at: '2026-07-21T00:10:00Z',
  manual_refresh_remaining_seconds: 0,
};

function deck(overrides: Partial<PlayerLoadout> = {}): PlayerLoadout {
  return {
    id: '1', deck_id: '1', deck_key: 'deck-1', champion_id: 2205, champion_name: 'Androxus',
    loadout_name: 'Main', card_ids: [1, 2, 3, 4, 5], card_levels: [5, 4, 3, 2, 1],
    talent_id: null, fetched_at: '2026-07-21T00:00:00Z', updated_at: '2026-07-21T00:00:00Z',
    ...overrides,
  };
}

function apiWith(cached: PlayerLoadout[], refresh: () => Promise<PlayerLoadoutsResponse>) {
  let refreshes = 0;
  const api = {
    resolvePlayer: async () => ({ id: '123', name: 'Player' }),
    playerLoadoutsById: async () => ({ loadouts: cached, freshness, refreshed: false }),
    refreshPlayerLoadoutsById: async () => { refreshes += 1; return refresh(); },
  } as unknown as PaladinsCatApi;
  return { api, refreshes: () => refreshes };
}

test('returns a matching cached champion without a vendor refresh', async () => {
  const fixture = apiWith([deck()], async () => { throw new Error('must not refresh'); });
  const result = await findPlayerChampionLoadouts(fixture.api, 'Player', 'androxus');
  assert.equal(result.loadouts.length, 1);
  assert.equal(result.refreshAttempted, false);
  assert.equal(fixture.refreshes(), 0);
});

test('refreshes once on a champion cache miss and serves the persisted result', async () => {
  const fixture = apiWith([], async () => ({ loadouts: [deck()], freshness, refreshed: true }));
  const result = await findPlayerChampionLoadouts(fixture.api, 'Player', 'Androxus');
  assert.equal(result.loadouts[0]?.loadout_name, 'Main');
  assert.equal(result.refreshed, true);
  assert.equal(fixture.refreshes(), 1);
});

test('a backend cooldown serves the database result instead of failing', async () => {
  const fixture = apiWith([deck({ champion_name: 'Strix' })], async () => {
    throw new PaladinsCatApiError('Refresh available in 42 seconds.', 429, 'LOADOUT_REFRESH_COOLDOWN');
  });
  const result = await findPlayerChampionLoadouts(fixture.api, 'Player', 'Androxus');
  assert.deepEqual(result.loadouts, []);
  assert.equal(result.refreshError, 'Refresh available in 42 seconds.');
  assert.equal(fixture.refreshes(), 1);
});
