export interface MatchRecord {
  match: {
    match_id: string;
    entry_datetime: string;
    queue_id: number;
    duration_seconds: number;
    region: string;
    map: string;
    team1_score: number | null;
    team2_score: number | null;
    winning_task_force: number;
    broken: boolean;
    recovered: boolean;
    private: boolean;
  };
  players: MatchPlayer[];
  bans?: Array<{ ban_slot?: number; champion_id: number; champion_name: string }>;
  facts?: MatchFactPlayer[];
}

export interface MatchFactPlayer {
  player_id: string;
  talents: Array<{
    talent_id: number;
    talent_name: string;
    champion_name?: string;
  }>;
}

export interface MatchPlayer {
  player_id: string;
  player_name: string;
  champion_id: number;
  champion_name: string;
  kills: number;
  deaths: number;
  assists: number;
  damage_done_physical: number;
  damage_done_in_hand?: number;
  damage_taken: number;
  damage_mitigated: number;
  healing: number;
  gold_earned: number;
  objective_assists?: number;
  final_match_level?: number;
  account_level?: number;
  party?: number;
  party_id?: number;
  party_number?: number;
  tier?: number;
  kbm_tier?: number;
  kbm_rank?: number;
  queue_elo?: number;
  win_status: string;
  task_force: number;
  league_tier: number;
  source: string;
  private_slot?: number;
  cheater?: boolean;
  sus_count?: number;
  verified?: boolean;
  profile_snapshot?: {
    level?: number | null;
    kbm_tier?: number | null;
    kbm_rank?: number | null;
    queue_elo?: number | null;
    cheater?: boolean;
    sus_count?: number | null;
    verified?: boolean;
  } | null;
}

export interface PlayerSearchResult {
  id: string;
  name: string;
  region?: string;
  platform?: string;
  kbm_tier?: number;
  kbm_points?: number;
  win_rate?: number | string;
}

export interface PlayerProfileResponse {
  player: Record<string, unknown> & { id: string; name: string };
  profileRefresh?: Record<string, unknown>;
  globalStats?: Record<string, unknown> | null;
  queueRatings?: Array<Record<string, unknown>>;
}

export interface Champion {
  id: number;
  name: string;
  title?: string;
  roles?: string;
}

export interface PlayerLoadout {
  id: string;
  deck_id: string | null;
  deck_key: string;
  champion_id: number;
  champion_name: string;
  loadout_name: string;
  card_ids: number[];
  card_levels: number[];
  talent_id: number | null;
  fetched_at: string;
  updated_at: string;
}

export interface PlayerLoadoutFreshness {
  ttl_seconds: number;
  refreshed_at: string | null;
  expires_at: string | null;
  remaining_seconds: number;
  expired: boolean;
  manual_refresh_available_at: string | null;
  manual_refresh_remaining_seconds: number;
}

export interface PlayerLoadoutsResponse {
  loadouts: PlayerLoadout[];
  freshness: PlayerLoadoutFreshness;
  refreshed: boolean;
  refresh_error?: string | null;
}

export interface LoadoutRenderRecord {
  player: PlayerSearchResult;
  loadout: PlayerLoadout;
}

export interface LoadoutCardAsset {
  id: number;
  name: string;
  description: string;
  shortDescription: string;
  championId: number;
  iconPath: string | null;
}
