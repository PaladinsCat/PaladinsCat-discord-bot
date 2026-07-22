export type ChampionLobbyScope = {
  value: string;
  label: string;
  tierMin?: number;
  tierMax?: number;
};

export const CHAMPION_LOBBY_SCOPES: readonly ChampionLobbyScope[] = [
  { value: 'global', label: 'Global ranked lobbies' },
  { value: 'bronze-gold', label: 'Bronze–Gold lobbies', tierMin: 1, tierMax: 15 },
  { value: 'platinum', label: 'Platinum+ lobbies', tierMin: 16, tierMax: 26 },
  { value: 'diamond', label: 'Diamond+ lobbies', tierMin: 21, tierMax: 26 },
];

export function championLobbyScope(value?: string | null): ChampionLobbyScope {
  return CHAMPION_LOBBY_SCOPES.find((scope) => scope.value === value) ?? CHAMPION_LOBBY_SCOPES[0]!;
}
