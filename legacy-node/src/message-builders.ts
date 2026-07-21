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

function jsonEmbed(title: string, json: unknown): DiscordMessagePayload {
  const text = JSON.stringify(json, null, 2).slice(0, 3500);
  return simpleEmbed(title, `\`\`\`json\n${text}\n\`\`\``);
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

export function buildCurrentPayload(result: Record<string, unknown>): DiscordMessagePayload {
  return jsonEmbed('Current match', result);
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
