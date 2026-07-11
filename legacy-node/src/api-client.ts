import type { Champion, MatchRecord, PlayerProfileResponse, PlayerSearchResult } from './types.js';

export class PaladinsCatApiError extends Error {
  constructor(message: string, public readonly status: number) {
    super(message);
  }
}

export class PaladinsCatApi {
  constructor(private readonly baseUrl: string, private readonly timeoutMs = 12000) {}

  private async get<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
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
    return this.get(`/players/${resolved.id}?include=ratings,champions`);
  }

  async playerHistory(input: string, limit = 10): Promise<Array<Record<string, unknown>>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/players/${resolved.id}/matches?limit=${limit}&refresh=false`);
  }

  async playerLoadouts(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/players/${resolved.id}/loadouts`);
  }

  async liveMatch(input: string): Promise<Record<string, unknown>> {
    const resolved = await this.resolvePlayer(input);
    return this.get(`/matches/live/${resolved.id}`);
  }

  async match(id: string): Promise<MatchRecord> {
    const payload = await this.get<{ matches: MatchRecord[] }>(`/matches/${encodeURIComponent(id)}`);
    const match = payload.matches?.[0];
    if (!match) throw new PaladinsCatApiError(`Match ${id} was not found`, 404);
    return match;
  }

  champions(): Promise<Champion[]> { return this.get('/champions'); }
  champion(idOrSlug: string): Promise<Record<string, unknown>> { return this.get(`/champions/${encodeURIComponent(idOrSlug)}`); }
  rankedLeaderboard(limit = 10): Promise<Array<Record<string, unknown>>> { return this.get(`/stats/ranked-leaderboard?top=${limit}`); }
  status(): Promise<Record<string, unknown>> { return this.get('/health'); }
}
