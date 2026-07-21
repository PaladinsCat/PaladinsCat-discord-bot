import type { APIEmbed, APIEmbedField } from 'discord.js';
import type { Champion, PlayerLoadout, PlayerProfileResponse } from './types.js';
import { assertDiscordMessage, type DiscordMessagePayload } from './discord-message.js';
import { buildPlayerProfileMessage } from './player-profile-message.js';

const accent = 0x2dd4a3;

function embedPayload(embed: APIEmbed): DiscordMessagePayload {
  return assertDiscordMessage({ embeds: [embed], allowedMentions: { parse: [] } });
}

function simpleEmbed(title: string, description: string, url?: string): DiscordMessagePayload {
  const embed: APIEmbed = { color: accent, title, description };
  if (url) embed.url = url;
  return embedPayload(embed);
}

export function buildHelpPayload(): DiscordMessagePayload {
  const embed: APIEmbed = {
    color: accent,
    title: 'PaladinsCat commands',
    description: [
      '`/player` profile, rank, record and performance',
      '`/match` optimized match-result image',
      '`/history` recent matches',
      '`/current` current live match',
      '`/loadout` choose and render a saved champion deck',
      '`/champion` champion ranked statistics',
      '`/leaderboard` top ranked players',
      '`/random` random champion by optional class',
      '`/status` API and renderer health',
    ].join('\n'),
  };
  return embedPayload(embed);
}

export function buildHistoryPayload(
  playerName: string,
  history: Array<Record<string, unknown>> | undefined,
  webUrl: string,
): DiscordMessagePayload {
  const rows = history ?? [];
  const lines = rows.slice(0, 10).map((row: Record<string, unknown>) => {
    const w = row.win_status === 'Winner' ? '✅' : '❌';
    const map = row.map ?? row.champion_name ?? '';
    const dur = row.duration_seconds ? `${Math.round(Number(row.duration_seconds) / 60)}m` : '';
    const region = row.region ?? '';
    const champ = row.champion_name ?? 'Unknown';
    const kda = `${row.kills ?? 0}/${row.deaths ?? 0}/${row.assists ?? 0}`;
    const id = row.match_id ?? '';
    const parts = [w, map, dur, region, champ, kda];
    return `${parts.filter(Boolean).join(' · ')} · [${id}](${webUrl}/matches/${id})`;
  });
  return embedPayload({
    color: accent,
    title: `${playerName} · Recent matches`,
    description: lines.join('\n') || 'No recent matches found.',
  });
}

const QUEUE_LABELS: Record<number, string> = {
  1: 'Casual Queue', 2: 'KBM', 4: '1v1', 8: 'Team Queue', 16: 'Open', 32: 'Doomspire',
  424: 'Casual Siege', 428: 'Ranked Siege (Controller)', 437: 'Casual Payload',
  451: 'PvE Survival', 452: 'Casual Onslaught', 469: 'Casual Team Deathmatch',
  474: 'Casual Battlegrounds Solo', 475: 'Casual Battlegrounds Duo',
  476: 'Casual Battlegrounds Quad', 486: 'Ranked Siege',
};
const TIER_NAMES = [
  'Unranked', 'Bronze V', 'Bronze IV', 'Bronze III', 'Bronze II', 'Bronze I',
  'Silver V', 'Silver IV', 'Silver III', 'Silver II', 'Silver I',
  'Gold V', 'Gold IV', 'Gold III', 'Gold II', 'Gold I',
  'Platinum V', 'Platinum IV', 'Platinum III', 'Platinum II', 'Platinum I',
  'Diamond V', 'Diamond IV', 'Diamond III', 'Diamond II', 'Diamond I', 'Master', 'Grandmaster',
];

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function cleanDiscordText(value: unknown, fallback: string): string {
  const text = String(value ?? '').trim() || fallback;
  return text.replace(/([\\`*_{}\[\]()#+\-.!|>~])/g, '\\$1');
}

function currentPlayerLine(player: Record<string, unknown>, sourcePlayerId: string, webUrl: string): string {
  const playerId = String(player.player_id ?? '');
  const playerName = cleanDiscordText(player.player_name, 'Private Account');
  const champion = cleanDiscordText(player.champion_name, 'Unknown champion');
  const tierNumber = Number(player.kbm_tier ?? player.live_tier ?? player.tier ?? 0);
  const tier = TIER_NAMES[Number.isInteger(tierNumber) ? tierNumber : 0] ?? 'Unranked';
  const name = /^\d+$/.test(playerId) && Number(playerId) > 0
    ? `[${playerName}](${webUrl}/players/${encodeURIComponent(playerId)})`
    : playerName;
  const marker = playerId === sourcePlayerId ? '▸ ' : '';
  return `${marker}**${champion}** · ${name}${tier === 'Unranked' ? '' : ` · ${tier}`}`;
}

export function buildCurrentPayload(result: Record<string, unknown>, webUrl: string): DiscordMessagePayload {
  const match = record(result.match);
  const players = Array.isArray(result.players) ? result.players.map(record) : [];
  const playerId = String(result.player_id ?? match.source_player_id ?? '');

  if (result.pending === true) {
    return embedPayload({
      color: 0xf0b232,
      title: 'Live lobby loading',
      description: 'The player is in a match, but the lobby snapshot is still being assembled. Try `/current` again shortly.',
      footer: { text: 'PaladinsCat refreshes pending live lobbies automatically.' },
    });
  }

  if (!match.match_id) {
    return embedPayload({
      color: 0x77808d,
      title: 'Not in a live match',
      description: 'No active Paladins match was found for this player.',
      footer: { text: 'Live status is cached briefly to protect the Paladins API.' },
    });
  }

  const matchId = String(match.match_id);
  const queueId = Number(match.queue_id ?? 0);
  const queue = QUEUE_LABELS[queueId] ?? (queueId > 0 ? `Queue #${queueId}` : 'Unknown queue');
  const map = cleanDiscordText(String(match.map ?? '').replace(/^(?:(?:live|ranked|wip)\s+)+/i, ''), 'Unknown map');
  const region = cleanDiscordText(match.region, 'Unknown region');
  const detectedAt = String(match.detected_at ?? '');
  const team = (taskForce: number) => players
    .filter((player) => Number(player.task_force) === taskForce)
    .map((player) => currentPlayerLine(player, playerId, webUrl))
    .join('\n') || 'Lobby details unavailable.';
  const embed: APIEmbed = {
    color: accent,
    title: `${map} · Live match`,
    description: `**${queue}** · ${region}\nMatch ID \`${matchId}\``,
    fields: [
      { name: 'Team 1', value: team(1), inline: true },
      { name: 'Team 2', value: team(2), inline: true },
    ],
    footer: { text: '▸ marks the requested player · Live lobby snapshot' },
  };
  if (!Number.isNaN(Date.parse(detectedAt))) embed.timestamp = new Date(detectedAt).toISOString();
  return embedPayload(embed);
}

export function buildLoadoutsPayload(
  playerName: string,
  loadouts: unknown[],
  webUrl: string,
  playerId?: string,
): DiscordMessagePayload {
  const lines = loadouts.slice(0, 15).map((row) => {
    const r = row as Record<string, unknown>;
    return `• **${r.champion_name ?? 'Champion'}** · ${r.loadout_name ?? 'Unnamed'}`;
  });
  const embed: APIEmbed = {
    color: accent,
    title: `${playerName} · Loadouts`,
    description: lines.join('\n') || 'No saved loadouts found.',
  };
  if (playerId) embed.url = `${webUrl}/players/${playerId}/loadouts`;
  return embedPayload(embed);
}

export function buildLoadoutSelectionPayload(
  playerName: string,
  championName: string,
  loadouts: PlayerLoadout[],
  webUrl: string,
  playerId: string,
  refreshed: boolean,
): DiscordMessagePayload {
  const count = loadouts.length;
  return embedPayload({
    color: accent,
    title: `${playerName} · ${championName}`,
    url: `${webUrl}/players/${playerId}/loadouts`,
    description: `Choose one of **${count}** saved loadout${count === 1 ? '' : 's'} below to generate its image.`,
    footer: { text: refreshed ? 'Saved loadouts refreshed from Paladins before this result.' : 'Served from the PaladinsCat database.' },
  });
}

export function buildNoLoadoutsPayload(
  playerName: string,
  championName: string,
  refreshError?: string | null,
): DiscordMessagePayload {
  const suffix = refreshError && !/cooldown/i.test(refreshError)
    ? '\nThe refresh did not complete, so this result may be stale.'
    : '';
  return simpleEmbed(
    `${playerName} · ${championName}`,
    `No saved loadouts found for this champion.${suffix}`,
  );
}

export function buildChampionPayload(
  result: Record<string, unknown>,
  webUrl: string,
): DiscordMessagePayload {
  const champion = (result.champion ?? {}) as Record<string, unknown>;
  const stats = (result.stats ?? {}) as Record<string, unknown>;
  const name = String(champion.name ?? 'Unknown');
  const numeric = (v: unknown): number | null => {
    const n = Number(v);
    return Number.isFinite(n) ? n : null;
  };
  return embedPayload({
    color: accent,
    title: name,
    url: `${webUrl}/champions/${encodeURIComponent(name.toLocaleLowerCase())}`,
    description: String(champion.title ?? ''),
    fields: [
      { name: 'Class', value: String(champion.roles ?? 'Unknown'), inline: true },
      { name: 'Win rate', value: numeric(stats.win_rate) == null ? '—' : `${Number(stats.win_rate).toFixed(1)}%`, inline: true },
      { name: 'Ranked matches', value: Number(stats.total_matches ?? 0).toLocaleString(), inline: true },
    ],
  });
}

export function buildLeaderboardPayload(
  rows: Array<Record<string, unknown>>,
  webUrl: string,
): DiscordMessagePayload {
  const lines = rows.map((row: Record<string, unknown>, index: number) => {
    const name = String(row.name ?? 'Unknown');
    const playerId = String(row.player_id ?? '');
    const points = Number(row.points ?? 0);
    return `**${index + 1}.** [${name}](${webUrl}/players/${playerId}) · ${points.toLocaleString()} TP`;
  });
  return embedPayload({
    color: accent,
    title: 'Ranked leaderboard',
    url: `${webUrl}/players/leaderboard`,
    description: lines.join('\n') || 'No ranked players found.',
  });
}

export function buildRandomPayload(
  champion: Champion,
  webUrl: string,
  role?: string,
): DiscordMessagePayload {
  const name = champion.name ?? 'Unknown';
  return embedPayload({
    color: accent,
    title: name,
    url: `${webUrl}/champions/${encodeURIComponent(name.toLocaleLowerCase())}`,
    description: `${champion.roles ?? 'Champion'}${role ? ` (${role})` : ''} · ${champion.title ?? ''}`,
  });
}

export function buildStatusPayload(
  apiStatus: Record<string, unknown>,
  latency: number,
  renderState: Record<string, unknown>,
): DiscordMessagePayload {
  function field(name: string, value: string, inline: boolean): APIEmbedField {
    return { name, value, inline };
  }
  return embedPayload({
    color: accent,
    title: 'PaladinsCat status',
    fields: [
      field('API', `${apiStatus.status ?? 'online'} · ${latency}ms`, true),
      field('Render queue', `${renderState.active ?? 0} active · ${renderState.queued ?? 0} queued`, true),
      field('Render cache', `${renderState.entries ?? 0} images · ${Number(renderState.bytes ?? 0) / 1048576} MiB`, true),
    ],
  });
}

export function buildPlayerHistoryPayload(
  player: { name: string; id: string },
  history: Array<Record<string, unknown>> | undefined,
  webUrl: string,
): DiscordMessagePayload {
  return buildHistoryPayload(player.name, history, webUrl);
}
