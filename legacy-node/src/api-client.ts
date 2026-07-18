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

  async discordPlayer(input: string, includeHistory = false): Promise<PlayerProfileResponse & { history?: Array<Record<string, unknown>> }> {
    const query = new URLSearchParams({ player: input });
    if (includeHistory) query.set('history', 'true');
    // This is intentionally the only bot read that is allowed to ask the
    // backend for Hi-Rez data. The endpoint owns the durable five-minute
    // profile/name lookup guard and the web's history-cache TTL.
    return this.get(`/players/discord?${query.toString()}`);
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
    return this.playerLoadoutsById(resolved.id);
  }

  async playerLoadoutsById(playerId: string): Promise<Record<string, unknown>> {
    return this.get(this.readPath(`/players/${encodeURIComponent(playerId)}/loadouts`));
  }

  async liveMatch(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/matches/live/${resolved.id}`);
  }

  async match(id: string): Promise<MatchRecord> {
    const matchPath = this.localOnly
      ? `/matches/batch?ids=${encodeURIComponent(id)}`
      : `/matches/${encodeURIComponent(id)}`;
    const [payload, facts] = await Promise.all([
      this.get<{ matches: MatchRecord[] }>(matchPath),
      this.get<{ players?: MatchFactPlayer[] }>(`/matches/fact/${encodeURIComponent(id)}`).catch(() => null),
    ]);
    const match = payload.matches?.[0];
    if (!match) throw new PaladinsCatApiError(`Match ${id} was not found`, 404);
    return {
      ...match,
      players: match.players.map((player) => this.hydrateMatchPlayer(player)),
      facts: facts?.players ?? [],
    };
  }

  private hydrateMatchPlayer(player: MatchPlayer): MatchPlayer {
    // The authoritative match read model already joins the current database
    // profile, ranked rating, moderation and verification state into this
    // snapshot. Reading ten individual player endpoints here added an N+1
    // request fan-out to every image without improving the displayed data.
    const snapshot = player.profile_snapshot ?? {};
    const numeric = (value: unknown): number | undefined => {
      const parsed = Number(value);
      return Number.isFinite(parsed) ? parsed : undefined;
    };
    return {
      ...player,
      final_match_level: numeric(snapshot.level) || numeric(player.final_match_level) || numeric(player.account_level) || 0,
      tier: numeric(snapshot.kbm_tier) ?? numeric(player.tier) ?? numeric(player.league_tier) ?? 0,
      kbm_tier: numeric(snapshot.kbm_tier) ?? numeric(player.kbm_tier),
      kbm_rank: numeric(snapshot.kbm_rank) ?? numeric(player.kbm_rank),
      queue_elo: numeric(snapshot.queue_elo) ?? numeric(player.queue_elo),
      cheater: Boolean(snapshot.cheater ?? player.cheater),
      sus_count: numeric(snapshot.sus_count) ?? numeric(player.sus_count) ?? 0,
      verified: Boolean(snapshot.verified ?? player.verified),
    };
  }

  champions(): Promise<Champion[]> { return this.get('/champions'); }
  champion(idOrSlug: string): Promise<Record<string, unknown>> { return this.get(`/champions/${encodeURIComponent(idOrSlug)}`); }
  rankedLeaderboard(limit = 10): Promise<Array<Record<string, unknown>>> { return this.get(`/stats/ranked-leaderboard?tier=26&top=${limit}`); }
  status(): Promise<Record<string, unknown>> { return this.get('/health'); }
}
