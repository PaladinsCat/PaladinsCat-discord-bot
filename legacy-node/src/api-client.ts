import type { Champion, DiscordSavedPlayer, MatchFactPlayer, MatchPlayer, MatchRecord, PlayerLoadout, PlayerLoadoutsResponse, PlayerProfileResponse, PlayerSearchResult } from './types.js';
import type { RankedLobbyScope } from './ranked-lobby.js';

export class PaladinsCatApiError extends Error {
  constructor(message: string, public readonly status: number, public readonly code?: string, public readonly details?: unknown) {
    super(message);
  }
}

export class PaladinsCatApi {
  private readonly localOnly: boolean;
  private readonly fetchImpl: typeof fetch;
  private readonly matchTimeoutMs: number;
  private readonly serviceToken?: string;

  constructor(
    private readonly baseUrl: string,
    private readonly timeoutMs = 12000,
    options: {
      localOnly?: boolean;
      fetchImpl?: typeof fetch;
      matchTimeoutMs?: number;
      serviceToken?: string;
    } = {},
  ) {
    this.localOnly = options.localOnly ?? false;
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.matchTimeoutMs = options.matchTimeoutMs ?? Math.max(timeoutMs, 125000);
    this.serviceToken = options.serviceToken?.trim() || undefined;
  }

  private readPath(path: string): string {
    if (!this.localOnly) return path;
    const separator = path.includes('?') ? '&' : '?';
    return `${path}${separator}refresh=false`;
  }

  private async request<T>(path: string, init: RequestInit = {}, timeoutMs = this.timeoutMs): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set('Accept', 'application/json');
    headers.set('User-Agent', 'PaladinsCatDiscordBot/0.1');
    if (this.serviceToken) headers.set('X-PaladinsCat-Service-Token', this.serviceToken);
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      ...init,
      headers,
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) {
      const payload = await response.json().catch(() => null) as { error?: { message?: string; code?: string; details?: unknown }; message?: string; code?: string; details?: unknown } | null;
      const apiError = payload?.error ?? payload;
      throw new PaladinsCatApiError(
        apiError?.message || `PaladinsCat API returned ${response.status}`,
        response.status,
        apiError?.code,
        apiError?.details,
      );
    }
    return response.json() as Promise<T>;
  }

  private get<T>(path: string, timeoutMs = this.timeoutMs): Promise<T> {
    return this.request<T>(path, {}, timeoutMs);
  }

  private post<T>(path: string, timeoutMs = this.timeoutMs): Promise<T> {
    return this.request<T>(path, { method: 'POST' }, timeoutMs);
  }

  private put<T>(path: string, body: unknown, timeoutMs = this.timeoutMs): Promise<T> {
    return this.request<T>(path, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }, timeoutMs);
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

  async savedDiscordPlayer(discordUserId: string): Promise<DiscordSavedPlayer> {
    const payload = await this.get<{ player: DiscordSavedPlayer }>(
      `/players/discord/saved-player?discordUserId=${encodeURIComponent(discordUserId)}`,
    );
    return payload.player;
  }

  async saveDiscordPlayer(discordUserId: string, input: string): Promise<DiscordSavedPlayer> {
    const resolved = await this.discordPlayer(input);
    const payload = await this.put<{ player: DiscordSavedPlayer }>('/players/discord/saved-player', {
      discordUserId,
      playerId: String(resolved.player.id),
    });
    return payload.player;
  }

  async playerHistory(input: string, limit = 10): Promise<Array<Record<string, unknown>>> {
    const resolved = await this.resolvePlayer(input);
    return this.playerHistoryById(resolved.id, limit);
  }

  async playerHistoryById(playerId: string, limit = 10): Promise<Array<Record<string, unknown>>> {
    return this.get(this.readPath(`/players/${encodeURIComponent(playerId)}/matches?limit=${limit}`));
  }

  async playerLoadouts(input: string): Promise<PlayerLoadoutsResponse> {
    const resolved = await this.resolvePlayer(input);
    return this.playerLoadoutsById(resolved.id);
  }

  async playerLoadoutsById(playerId: string): Promise<PlayerLoadoutsResponse> {
    return this.get(this.readPath(`/players/${encodeURIComponent(playerId)}/loadouts`));
  }

  async refreshPlayerLoadoutsById(playerId: string): Promise<PlayerLoadoutsResponse> {
    return this.post(`/players/${encodeURIComponent(playerId)}/loadouts/refresh`);
  }

  async playerLoadoutById(playerId: string, loadoutId: string | number): Promise<PlayerLoadout> {
    const payload = await this.get<{ loadout: PlayerLoadout }>(
      this.readPath(`/players/${encodeURIComponent(playerId)}/loadouts/decks/${encodeURIComponent(loadoutId)}`),
    );
    return payload.loadout;
  }

  async liveMatch(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/live/players/${resolved.id}`);
  }

  async match(id: string): Promise<MatchRecord> {
    const encodedId = encodeURIComponent(id);
    const readFacts = () => this.get<{ players?: MatchFactPlayer[] }>(`/matches/fact/${encodedId}`).catch(() => null);
    let payload: { matches: MatchRecord[] };
    let facts: { players?: MatchFactPlayer[] } | null;

    if (this.localOnly) {
      // Keep the common path database-only and cheap. On a true miss, the
      // direct endpoint owns requested-match ingestion, waits for durable
      // completion, and returns the newly persisted read model.
      [payload, facts] = await Promise.all([
        this.get<{ matches: MatchRecord[] }>(`/matches/batch?ids=${encodedId}`),
        readFacts(),
      ]);
      if (!payload.matches?.[0]) {
        payload = await this.get<{ matches: MatchRecord[] }>(`/matches/${encodedId}`, this.matchTimeoutMs);
        // The speculative facts read normally returned 404 before ingestion.
        // Retry only after the completion boundary so the first image includes
        // the same talents/items facts that subsequent web reads receive.
        facts = await readFacts();
      }
    } else {
      [payload, facts] = await Promise.all([
        this.get<{ matches: MatchRecord[] }>(`/matches/${encodedId}`, this.matchTimeoutMs),
        readFacts(),
      ]);
      if (!facts && payload.matches?.[0]) facts = await readFacts();
    }

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
  championPageData(idOrSlug: string, scope: RankedLobbyScope): Promise<Record<string, unknown>> {
    const query = new URLSearchParams();
    if (scope.tierMin != null) query.set('tierMin', String(scope.tierMin));
    if (scope.tierMax != null) query.set('tierMax', String(scope.tierMax));
    const suffix = query.size > 0 ? `?${query.toString()}` : '';
    return this.get(`/champions/${encodeURIComponent(idOrSlug)}/page-data${suffix}`);
  }

  rankedMaps(limit = 100): Promise<Array<Record<string, unknown>>> {
    return this.get(`/stats/maps?queueId=486&limit=${Math.min(Math.max(limit, 1), 100)}`);
  }

  async rankedCompositions(limit = 5): Promise<Array<Record<string, unknown>>> {
    const payload = await this.get<{ data?: Array<Record<string, unknown>> } | Array<Record<string, unknown>>>(
      `/matches/compositions?sortBy=count&order=desc&limit=${Math.min(Math.max(limit, 1), 25)}`,
    );
    return Array.isArray(payload) ? payload : payload.data ?? [];
  }

  rankedItems(scope: RankedLobbyScope, limit = 20): Promise<Array<Record<string, unknown>>> {
    const query = new URLSearchParams({ mode: 'ranked', limit: String(Math.min(Math.max(limit, 1), 50)) });
    if (scope.tierMin != null) query.set('tierMin', String(scope.tierMin));
    if (scope.tierMax != null) query.set('tierMax', String(scope.tierMax));
    return this.get(`/stats/items?${query.toString()}`);
  }
}
