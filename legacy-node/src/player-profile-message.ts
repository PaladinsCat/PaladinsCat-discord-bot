import { escapeMarkdown, type APIEmbed, type APIEmbedField } from 'discord.js';
import { assertDiscordMessage, type DiscordMessagePayload } from './discord-message.js';
import type { PlayerProfileResponse } from './types.js';

const accent = 0x2dd4a3;
export const DEFAULT_PLAYER_AVATAR_PATH = '/images/icons/Avatar_Default_Icon.png';

function number(value: unknown): number | null {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function integer(value: unknown): number | null {
  const parsed = number(value);
  return parsed == null ? null : Math.trunc(parsed);
}

function compact(value: unknown, maxLength: number): string {
  const plain = String(value ?? '').replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim();
  if (!plain) return '';
  return escapeMarkdown(plain).slice(0, maxLength);
}

function formatNumber(value: unknown): string {
  const parsed = number(value);
  return parsed == null ? '—' : parsed.toLocaleString();
}

function formatPercent(wins: unknown, losses: unknown): string | null {
  const winValue = number(wins) ?? 0;
  const lossValue = number(losses) ?? 0;
  const games = winValue + lossValue;
  return games > 0 ? `${((winValue / games) * 100).toFixed(1)}%` : null;
}

function playerAvatarUrl(value: unknown, webUrl: string): string {
  const rawUrl = String(value ?? '').trim();
  if (/^https?:\/\//i.test(rawUrl)) return rawUrl;
  return `${webUrl.replace(/\/+$/, '')}${DEFAULT_PLAYER_AVATAR_PATH}`;
}

function tierName(tier: unknown, rank: unknown): string {
  const value = integer(tier) ?? 0;
  const leaderboardRank = integer(rank) ?? 0;
  if (value === 26 && leaderboardRank > 0 && leaderboardRank <= 100) return `Grandmaster #${leaderboardRank}`;
  if (value === 26) return leaderboardRank > 100 ? `Master #${leaderboardRank - 100}` : 'Master';
  const names: Record<number, string> = {
    1: 'Bronze V', 2: 'Bronze IV', 3: 'Bronze III', 4: 'Bronze II', 5: 'Bronze I',
    6: 'Silver V', 7: 'Silver IV', 8: 'Silver III', 9: 'Silver II', 10: 'Silver I',
    11: 'Gold V', 12: 'Gold IV', 13: 'Gold III', 14: 'Gold II', 15: 'Gold I',
    16: 'Platinum V', 17: 'Platinum IV', 18: 'Platinum III', 19: 'Platinum II', 20: 'Platinum I',
    21: 'Diamond V', 22: 'Diamond IV', 23: 'Diamond III', 24: 'Diamond II', 25: 'Diamond I',
  };
  return names[value] ?? 'Unranked';
}

function formatQueue(label: string, tier: unknown, rank: unknown, points: unknown, wins: unknown, losses: unknown): APIEmbedField | null {
  const value = integer(tier) ?? 0;
  const games = (number(wins) ?? 0) + (number(losses) ?? 0);
  if (value <= 0 && games <= 0 && (number(points) ?? 0) <= 0) return null;
  const record = games > 0 ? `\n${formatNumber(wins)}W – ${formatNumber(losses)}L` : '';
  const tp = (number(points) ?? 0) > 0 ? ` · ${formatNumber(points)} TP` : '';
  return { name: label, value: `${tierName(tier, rank)}${tp}${record}`, inline: true };
}

function topChampionField(rows: Array<Record<string, unknown>>): APIEmbedField | null {
  const top = rows
    .map((row) => ({
      name: compact(row.champion_name ?? row.name, 42),
      rating: number(row.elo ?? row.mu ?? row.rating),
      matches: number(row.matches_played ?? row.total_matches),
    }))
    .filter((row) => row.name && (row.rating != null || (row.matches ?? 0) > 0))
    .sort((left, right) => (right.rating ?? 0) - (left.rating ?? 0) || (right.matches ?? 0) - (left.matches ?? 0))
    .slice(0, 3);
  if (top.length === 0) return null;
  return {
    name: 'Top champions',
    value: top.map((row) => `${row.name}${row.rating != null ? ` · ${Math.round(row.rating).toLocaleString()} ELO` : ''}`).join('\n'),
    inline: true,
  };
}

function performanceField(player: Record<string, unknown>): APIEmbedField | null {
  const metrics = [
    ['DPM', player.avg_dpm], ['HPM', player.avg_hpm], ['MPM', player.avg_mpm], ['EGPM', player.avg_egpm],
  ]
    .map(([label, value]) => ({ label, value: number(value) }))
    .filter((metric): metric is { label: string; value: number } => metric.value != null && metric.value > 0);
  if (metrics.length === 0) return null;
  return { name: 'Ranked performance', value: metrics.map((metric) => `${metric.label} ${Math.round(metric.value).toLocaleString()}`).join(' · '), inline: true };
}

export function buildPlayerProfileMessage(
  response: PlayerProfileResponse,
  webUrl: string,
): DiscordMessagePayload {
  const player = response.player;
  const playerId = encodeURIComponent(String(player.id));
  const playerName = compact(player.name, 256) || 'Unknown player';
  const context = [
    number(player.level) != null ? `Level ${formatNumber(player.level)}` : '',
    compact(player.region, 40),
    compact(player.platform, 40),
  ].filter(Boolean).join(' • ');
  const title = compact(player.title, 220);
  const description = [context ? `**${context}**` : '', title ? `*${title}*` : ''].filter(Boolean).join('\n');
  const fields: APIEmbedField[] = [];

  const kbm = formatQueue('Ranked KBM', player.kbm_tier, player.kbm_rank, player.kbm_points, player.kbm_wins, player.kbm_losses);
  const controller = formatQueue('Ranked Controller', player.controller_tier, player.controller_rank, player.controller_points, player.controller_wins, player.controller_losses);
  if (kbm) fields.push(kbm);
  if (controller) fields.push(controller);

  const record = formatPercent(player.wins, player.losses);
  fields.push({
    name: 'Account record',
    value: `${formatNumber(player.wins)}W – ${formatNumber(player.losses)}L${record ? `\n${record} win rate` : ''}`,
    inline: true,
  });

  const playtime = number(player.hours_played);
  if (playtime != null && playtime > 0) fields.push({ name: 'Playtime', value: `${Math.round(playtime).toLocaleString()} hours`, inline: true });

  const performance = performanceField(player);
  if (performance) fields.push(performance);
  const champions = topChampionField(response.championRatings ?? []);
  if (champions) fields.push(champions);

  const refreshedAt = new Date(String(response.profileRefresh?.refreshed_at ?? player.last_updated ?? ''));
  const timestamp = Number.isFinite(refreshedAt.getTime()) ? refreshedAt.toISOString() : undefined;
  const embed: APIEmbed = {
    color: accent,
    author: { name: 'PaladinsCat Player Profile' },
    title: playerName,
    url: `${webUrl}/players/${playerId}`,
    description: description || undefined,
    fields,
    thumbnail: { url: playerAvatarUrl(player.avatar_url, webUrl) },
    footer: { text: 'PaladinsCat' },
    timestamp,
  };

  return assertDiscordMessage({ embeds: [embed], allowedMentions: { parse: [] } });
}
