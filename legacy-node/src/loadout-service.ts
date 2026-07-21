import { PaladinsCatApi, PaladinsCatApiError } from './api-client.js';
import type { PlayerLoadout, PlayerLoadoutFreshness, PlayerSearchResult } from './types.js';

export function normalizeChampionName(value: string): string {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export interface PlayerChampionLoadouts {
  player: PlayerSearchResult;
  championName: string;
  loadouts: PlayerLoadout[];
  freshness: PlayerLoadoutFreshness;
  refreshAttempted: boolean;
  refreshed: boolean;
  refreshError: string | null;
}

/**
 * Read saved decks from Postgres first. A missing champion is the only reason
 * to ask the backend for a refresh; the backend owns the durable per-player
 * ten-minute guard and persists the vendor result before this method returns.
 */
export async function findPlayerChampionLoadouts(
  api: PaladinsCatApi,
  playerInput: string,
  championInput: string,
): Promise<PlayerChampionLoadouts> {
  const player = await api.resolvePlayer(playerInput.trim());
  const requestedChampion = normalizeChampionName(championInput);
  if (!requestedChampion) throw new Error('Enter a champion name.');

  const cached = await api.playerLoadoutsById(player.id);
  const filter = (rows: PlayerLoadout[]) => rows.filter(
    (row) => normalizeChampionName(row.champion_name) === requestedChampion,
  );
  let matching = filter(cached.loadouts);
  const knownChampionName = matching[0]?.champion_name
    ?? cached.loadouts.find((row) => normalizeChampionName(row.champion_name) === requestedChampion)?.champion_name
    ?? championInput.trim();
  if (matching.length > 0) {
    return {
      player,
      championName: knownChampionName,
      loadouts: matching,
      freshness: cached.freshness,
      refreshAttempted: false,
      refreshed: false,
      refreshError: cached.refresh_error ?? null,
    };
  }

  try {
    const refreshed = await api.refreshPlayerLoadoutsById(player.id);
    matching = filter(refreshed.loadouts);
    return {
      player,
      championName: matching[0]?.champion_name ?? knownChampionName,
      loadouts: matching,
      freshness: refreshed.freshness,
      refreshAttempted: true,
      refreshed: refreshed.refreshed,
      refreshError: refreshed.refresh_error ?? null,
    };
  } catch (error) {
    // A cooldown means another recent command already paid for a vendor read.
    // The cached result remains authoritative and should still be served.
    if (error instanceof PaladinsCatApiError && error.status === 429) {
      return {
        player,
        championName: knownChampionName,
        loadouts: matching,
        freshness: cached.freshness,
        refreshAttempted: true,
        refreshed: false,
        refreshError: error.message,
      };
    }
    throw error;
  }
}
