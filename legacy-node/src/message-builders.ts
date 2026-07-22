import type { APIEmbed } from 'discord.js';
import type { PlayerLoadout, PlayerProfileResponse } from './types.js';
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
      '`/champion` database-backed ranked statistics by lobby tier',
      '`/maps` statistics for every ranked map',
      '`/composition` five most-played ranked team compositions',
      '`/items` ranked item usage and win rate by lobby tier',
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

function numericMetric(value: unknown): number | null {
  if (value == null || value === '') return null;
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

function estimateLiveTeamWinChance(players: Record<string, unknown>[]): { teamOne: number; teamTwo: number } | null {
  const teamMetrics = (taskForce: number) => {
    const team = players.filter((player) => Number(player.task_force) === taskForce);
    const elos = team.flatMap((player) => {
      const value = numericMetric(player.queue_elo);
      return value != null && value > 0 && value <= 3500 ? [value] : [];
    });
    const winRates = team.flatMap((player) => {
      const value = numericMetric(player.profile_win_rate);
      return value != null && value >= 0 && value <= 100 ? [value] : [];
    });
    const minimumCoverage = Math.min(3, team.length);
    return {
      averageElo: elos.length >= minimumCoverage
        ? elos.reduce((sum, value) => sum + value, 0) / elos.length
        : null,
      averageWinRate: winRates.length >= minimumCoverage
        ? winRates.reduce((sum, value) => sum + value, 0) / winRates.length
        : null,
    };
  };

  const teamOne = teamMetrics(1);
  const teamTwo = teamMetrics(2);
  if (teamOne.averageElo == null || teamTwo.averageElo == null) return null;

  // Queue ELO is the primary matchup signal. Global win rate provides a small
  // calibration only when both teams have enough PaladinsCat profile history.
  const eloProbability = 1 / (1 + 10 ** ((teamTwo.averageElo - teamOne.averageElo) / 400));
  const winRateProbability = teamOne.averageWinRate != null && teamTwo.averageWinRate != null
    ? teamOne.averageWinRate / (teamOne.averageWinRate + teamTwo.averageWinRate || 100)
    : 0.5;
  const blended = Math.min(0.85, Math.max(0.15, eloProbability * 0.85 + winRateProbability * 0.15));
  const teamOnePercent = Math.round(blended * 100);
  return { teamOne: teamOnePercent, teamTwo: 100 - teamOnePercent };
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
  const globalWinRate = numericMetric(player.profile_win_rate);
  const queueElo = numericMetric(player.queue_elo);
  const details = [
    tier === 'Unranked' ? null : tier,
    globalWinRate == null ? null : `Global ${globalWinRate.toFixed(1)}% WR`,
    queueElo == null ? null : `${Math.round(queueElo).toLocaleString('en-US')} ELO`,
  ].filter((value): value is string => Boolean(value));
  return `${marker}**${champion}** · ${name}${details.length > 0 ? ` · ${details.join(' · ')}` : ''}`;
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
  const estimate = estimateLiveTeamWinChance(players);
  const team = (taskForce: number) => players
    .filter((player) => Number(player.task_force) === taskForce)
    .map((player) => currentPlayerLine(player, playerId, webUrl))
    .join('\n') || 'Lobby details unavailable.';
  const embed: APIEmbed = {
    color: accent,
    title: `${map} · Live match`,
    description: `**${queue}** · ${region}\nMatch ID \`${matchId}\``,
    fields: [
      { name: estimate ? `Team 1 · ${estimate.teamOne}% win chance` : 'Team 1', value: team(1), inline: true },
      { name: estimate ? `Team 2 · ${estimate.teamTwo}% win chance` : 'Team 2', value: team(2), inline: true },
    ],
    footer: {
      text: `${estimate ? 'Estimate blends queue ELO with global win rate · ' : ''}▸ marks the requested player · Live lobby snapshot`,
    },
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
  lobbyLabel = 'Global ranked lobbies',
): DiscordMessagePayload {
  const champion = record(result.champion);
  const stats = record(result.stats);
  const performance = record(result.championPerformance);
  const talentStats = record(result.talentStats);
  const identityMetric = ['dpm', 'wpm', 'apm', 'gpm', 'hpm', 'mpm', 'kda']
    .map((key) => record(performance[key]))
    .find((metric) => metric.championName || metric.className) ?? {};
  const name = String(champion.name ?? identityMetric.championName ?? 'Unknown');
  const className = String(champion.roles ?? identityMetric.className ?? 'Unknown');
  const formattedNumber = (value: unknown, decimals = 0): string => {
    const numeric = numericMetric(value);
    return numeric == null ? '—' : numeric.toLocaleString(undefined, {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals,
    });
  };
  const averageTier = numericMetric(stats.avg_league_tier);
  const roundedTier = averageTier == null ? 0 : Math.max(0, Math.min(TIER_NAMES.length - 1, Math.round(averageTier)));
  const tierValue = averageTier == null || averageTier <= 0
    ? '—'
    : `**${TIER_NAMES[roundedTier]}**\n${averageTier.toFixed(1)} average`;
  const winRate = numericMetric(stats.win_rate);
  const recordValue = [
    winRate == null ? '**—** win rate' : `**${winRate.toFixed(1)}%** win rate`,
    `${formattedNumber(stats.wins)} W · ${formattedNumber(stats.losses)} L`,
    `${formattedNumber(stats.total_plays ?? stats.total_matches)} total plays`,
  ].join('\n');
  const metricFields = [
    ['DPM', 'dpm', 0], ['WPM', 'wpm', 0], ['APM', 'apm', 0], ['CPM', 'gpm', 0],
    ['HPM', 'hpm', 0], ['SPM', 'mpm', 0], ['KDA', 'kda', 1],
  ].map(([label, key, decimals]) => {
    const metric = record(performance[String(key)]);
    const p10 = formattedNumber(metric.p10, Number(decimals));
    const p90 = formattedNumber(metric.p90, Number(decimals));
    return {
      name: String(label),
      value: `**${formattedNumber(metric.avgValue, Number(decimals))}**\nP10–P90 ${p10}–${p90}`,
      inline: true,
    };
  });
  const talentCoverage = Math.max(0, numericMetric(talentStats.talentCoveredMatches) ?? 0);
  const talents = Array.isArray(talentStats.talents)
    ? (talentStats.talents as Array<Record<string, unknown>>)
      .slice()
      .sort((left, right) => (numericMetric(right.totalPlays) ?? 0) - (numericMetric(left.totalPlays) ?? 0))
      .slice(0, 3)
    : [];
  const talentValue = talents.map((talent) => {
    const plays = numericMetric(talent.totalPlays) ?? 0;
    const pickRate = talentCoverage > 0 ? (100 * plays) / talentCoverage : null;
    return `**${cleanDiscordText(talent.talentName, 'Unknown')}** · ${formattedNumber(talent.winRate, 1)}% WR · ${pickRate == null ? '—' : `${pickRate.toFixed(1)}%`} pick · ${formattedNumber(plays)} plays`;
  }).join('\n') || 'No ranked talent data in this lobby range.';

  return embedPayload({
    color: accent,
    title: `${name} · Ranked performance`,
    url: `${webUrl}/champions/${encodeURIComponent(name.toLocaleLowerCase())}`,
    description: `**${lobbyLabel}** · Served from the PaladinsCat champion database.`,
    fields: [
      { name: 'Class', value: className, inline: true },
      { name: 'Average lobby tier', value: tierValue, inline: true },
      { name: 'Ranked record', value: recordValue, inline: true },
      ...metricFields,
      { name: 'Most played talents', value: talentValue },
    ],
    footer: { text: 'Lobby filters use the ranked match database; global is the default.' },
  });
}

function descriptionFromLines(lines: string[], fallback: string): string {
  const kept: string[] = [];
  let length = 0;
  for (const line of lines) {
    const nextLength = length + line.length + (kept.length > 0 ? 1 : 0);
    if (nextLength > 4000) break;
    kept.push(line);
    length = nextLength;
  }
  return kept.join('\n') || fallback;
}

function durationLabel(value: unknown): string {
  const seconds = Math.max(0, Math.round(numericMetric(value) ?? 0));
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`;
}

export function buildMapsPayload(
  rows: Array<Record<string, unknown>>,
  webUrl: string,
): DiscordMessagePayload {
  const lines = rows.map((row) => {
    const name = String(row.map ?? 'Unknown');
    const safeName = cleanDiscordText(name.replace(/^Ranked\s+/i, ''), 'Unknown');
    const matches = Math.max(0, Math.round(numericMetric(row.total_matches) ?? 0));
    const share = numericMetric(row.distribution_rate);
    return `**[${safeName}](${webUrl}/game/maps/${encodeURIComponent(name)})** · ${matches.toLocaleString()} matches · ${share == null ? '—' : `${share.toFixed(1)}%`} of pool · ${durationLabel(row.avg_duration_seconds)} avg`;
  });
  return embedPayload({
    color: accent,
    title: 'Ranked map statistics',
    url: `${webUrl}/game/maps`,
    description: descriptionFromLines(lines, 'No ranked map statistics are available.'),
    footer: { text: 'PaladinsCat ranked match database · Ordered by matches played' },
  });
}

export function buildCompositionPayload(
  rows: Array<Record<string, unknown>>,
  webUrl: string,
): DiscordMessagePayload {
  const lines = rows.slice(0, 5).map((row, index) => {
    const roles = [
      `${Math.round(numericMetric(row.frontline) ?? 0)} Frontline`,
      `${Math.round(numericMetric(row.damage) ?? 0)} Damage`,
      `${Math.round(numericMetric(row.flank) ?? 0)} Flank`,
      `${Math.round(numericMetric(row.support) ?? 0)} Support`,
    ].join(' · ');
    const matches = Math.max(0, Math.round(numericMetric(row.count) ?? 0));
    const winRate = numericMetric(row.winrate);
    return `**${index + 1}. ${roles}**\n${matches.toLocaleString()} matches · ${winRate == null ? '—' : `${winRate.toFixed(1)}%`} win rate`;
  });
  return embedPayload({
    color: accent,
    title: 'Top ranked team compositions',
    url: `${webUrl}/game/compositions`,
    description: descriptionFromLines(lines, 'No ranked composition statistics are available.'),
    footer: { text: 'Top five by matches played · PaladinsCat ranked match database' },
  });
}

export function buildItemsPayload(
  rows: Array<Record<string, unknown>>,
  webUrl: string,
  lobbyLabel = 'Global ranked lobbies',
): DiscordMessagePayload {
  const lines = rows.slice(0, 20).map((row, index) => {
    const id = String(row.item_id ?? '');
    const name = cleanDiscordText(row.item_name, 'Unknown item');
    const uses = Math.max(0, Math.round(numericMetric(row.total_uses ?? row.total_usage) ?? 0));
    const pickRate = numericMetric(row.pick_rate);
    const winRate = numericMetric(row.win_rate);
    const linkedName = id ? `[${name}](${webUrl}/game/items/${encodeURIComponent(id)})` : name;
    return `**${index + 1}. ${linkedName}** · ${pickRate == null ? '—' : `${pickRate.toFixed(1)}%`} pick · ${winRate == null ? '—' : `${winRate.toFixed(1)}%`} WR · ${uses.toLocaleString()} uses`;
  });
  return embedPayload({
    color: accent,
    title: 'Ranked item statistics',
    url: `${webUrl}/game/items`,
    description: `**${lobbyLabel}**\n${descriptionFromLines(lines, 'No ranked item statistics are available.')}`,
    footer: { text: 'Top twenty by usage · Global ranked lobbies are the default' },
  });
}

export function buildPlayerHistoryPayload(
  player: { name: string; id: string },
  history: Array<Record<string, unknown>> | undefined,
  webUrl: string,
): DiscordMessagePayload {
  return buildHistoryPayload(player.name, history, webUrl);
}
