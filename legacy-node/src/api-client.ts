import type { Champion, MatchFactPlayer, MatchPlayer, MatchRecord, PlayerProfileResponse, PlayerSearchResult } from './types.js';

export class PaladinsCatApiError extends Error {
  constructor(message: string, public readonly status: number) {
    super(message);
  }
}

export class PaladinsCatApi {
  private readonly localOnly: boolean;
  private readonly fetchImpl: typeof fetch;

  constructor(
    private readonly baseUrl: string,
    private readonly timeoutMs = 12000,
    options: { localOnly?: boolean; fetchImpl?: typeof fetch } = {},
  ) {
    this.localOnly = options.localOnly ?? false;
    this.fetchImpl = options.fetchImpl ?? fetch;
  }

  private readPath(path: string): string {
    if (!this.localOnly) return path;
    const separator = path.includes('?') ? '&' : '?';
    return `${path}${separator}refresh=false`;
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      headers: { Accept: 'application/json', 'User-Agent': 'PaladinsCatDiscordBot/0.1' },
      signal: AbortSignal.timeout(this.timeoutMs),
    });
    if (!response.ok) throw new PaladinsCatApiError(`PaladinsCat API returned ${response.status}`, response.status);
    return response.json() as Promise<T>;
  }

  async searchPlayers(name: string, limit = 5): Promise<PlayerSearchResult[]> {
    return this.get(`/players/search?name=${encodeURIComponent(name)}&limit=${limit}`);
  }

  async resolvePlayer(input: string): Promise<PlayerSearchResult> {
    if (/^\d+$/.test(input)) return { id: input, name: input };
    const rows = await this.searchPlayers(input, 5);
    const exact = rows.find((row) => row.name.toLocaleLowerCase() === input.toLocaleLowerCase());
    const result = exact ?? rows[0];
    if (!result) throw new PaladinsCatApiError(`Player “${input}” was not found`, 404);
    return result;
  }

  async player(input: string): Promise<PlayerProfileResponse> {
    const resolved = await this.resolvePlayer(input);
    return this.playerById(resolved.id);
  }

  async playerById(playerId: string): Promise<PlayerProfileResponse> {
    return this.get(this.readPath(`/players/${encodeURIComponent(playerId)}?include=ratings`));
  }

  async playerHistory(input: string, limit = 10): Promise<Array<Record<string, unknown>>> {
    const resolved = await this.resolvePlayer(input);
    return this.playerHistoryById(resolved.id, limit);
  }

  async playerHistoryById(playerId: string, limit = 10): Promise<Array<Record<string, unknown>>> {
    return this.get(this.readPath(`/players/${encodeURIComponent(playerId)}/matches?limit=${limit}`));
  }

  async playerLoadouts(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(this.readPath(`/players/${resolved.id}/loadouts`));
  }

  async liveMatch(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/matches/live/${resolved.id}`);
  }

  async match(id: string): Promise<MatchRecord> {
    const [payload, facts] = await Promise.all([
      this.get<{ matches: MatchRecord[] }>(this.readPath(`/matches/${encodeURIComponent(id)}`)),
      this.get<{ players?: MatchFactPlayer[] }>(`/matches/fact/${encodeURIComponent(id)}`).catch(() => null),
    ]);
    const match = payload.matches?.[0];
    if (!match) throw new PaladinsCatApiError(`Match ${id} was not found`, 404);
    const profiles = await Promise.all(match.players.map(async (player) => {
      try {
        const profile = await this.get<PlayerProfileResponse>(this.readPath(`/players/${player.player_id}?include=ratings`));
        return this.hydrateMatchPlayer(player, profile, match.match.queue_id);
      } catch {
        return {
          ...player,
          verified: Boolean(player.verified ?? player.profile_snapshot?.verified),
        };
      }
    }));
    return { ...match, players: profiles, facts: facts?.players ?? [] };
  }

  private hydrateMatchPlayer(player: MatchPlayer, response: PlayerProfileResponse, queueId: number): MatchPlayer {
    const profile = response.player ?? {};
    const queueRatings = response.queueRatings ?? [];
    const rating = queueRatings.find((row) => Number(row.queue_id) === queueId)
      ?? queueRatings.find((row) => Number(row.queue_id) === 486)
      ?? queueRatings[0];
    const numeric = (value: unknown): number | undefined => {
      const parsed = Number(value);
      return Number.isFinite(parsed) ? parsed : undefined;
    };
    return {
      ...player,
      // Match payload levels are capped at 999. Prefer the refreshed profile's
      // XP-derived level whenever it is available.
      final_match_level: numeric(profile.level) || numeric(player.final_match_level) || numeric(player.account_level) || 0,
      tier: numeric(profile.kbm_tier) ?? numeric(player.tier) ?? numeric(player.league_tier) ?? 0,
      kbm_tier: numeric(profile.kbm_tier),
      kbm_rank: numeric(profile.kbm_rank),
      queue_elo: numeric(rating?.mu),
      cheater: Boolean(profile.cheater),
      sus_count: numeric(profile.sus_count) ?? 0,
      verified: Boolean(profile.verified ?? player.verified ?? player.profile_snapshot?.verified),
    };
  }

  champions(): Promise<Champion[]> { return this.get('/champions'); }
  champion(idOrSlug: string): Promise<Record<string, unknown>> { return this.get(`/champions/${encodeURIComponent(idOrSlug)}`); }
  rankedLeaderboard(limit = 10): Promise<Array<Record<string, unknown>>> { return this.get(`/stats/ranked-leaderboard?tier=26&top=${limit}`); }
  status(): Promise<Record<string, unknown>> { return this.get('/health'); }
}
