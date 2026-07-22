export type RankedLobbyScope = {
  value: string;
  label: string;
  tierMin?: number;
  tierMax?: number;
};

export const RANKED_LOBBY_SCOPES: readonly RankedLobbyScope[] = [
  { value: 'global', label: 'Global ranked lobbies' },
  { value: 'bronze-gold', label: 'Bronze–Gold lobbies', tierMin: 1, tierMax: 15 },
  { value: 'platinum', label: 'Platinum+ lobbies', tierMin: 16, tierMax: 26 },
  { value: 'diamond', label: 'Diamond+ lobbies', tierMin: 21, tierMax: 26 },
];

export function rankedLobbyScope(value?: string | null): RankedLobbyScope {
  return RANKED_LOBBY_SCOPES.find((scope) => scope.value === value) ?? RANKED_LOBBY_SCOPES[0]!;
}
