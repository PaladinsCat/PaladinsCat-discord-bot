export interface MatchRecord {
  match: {
    match_id: string;
    entry_datetime: string;
    queue_id: number;
    duration_seconds: number;
    region: string;
    map: string;
    team1_score: number;
    team2_score: number;
    winning_task_force: number;
    broken: boolean;
    recovered: boolean;
    private: boolean;
  };
  players: MatchPlayer[];
  bans?: Array<{ champion_id: number; champion_name: string }>;
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
  win_status: string;
  task_force: number;
  league_tier: number;
  source: string;
  private_slot?: number;
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
  queueRatings?: Array<Record<string, unknown>>;
  championRatings?: Array<Record<string, unknown>>;
}

export interface Champion {
  id: number;
  name: string;
  title?: string;
  roles?: string;
}
