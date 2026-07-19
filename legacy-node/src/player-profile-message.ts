import { escapeMarkdown, type APIEmbed, type APIEmbedField } from 'discord.js';
import { assertDiscordMessage, type DiscordMessagePayload } from './discord-message.js';
import { canonicalAvatarAssetUrl } from './paladins-avatar-assets.js';
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

function codeBlock(lines: string[]): string {
  return `\`\`\`\n${lines.join('\n')}\n\`\`\``;
}

function statLine(label: string, value: string): string {
  return `${label.padEnd(14)}: ${value}`;
}

function formatPlaytime(hours: unknown, minutes: unknown): string | null {
  const totalHours = number(hours) ?? ((number(minutes) ?? 0) / 60);
  if (!Number.isFinite(totalHours) || totalHours <= 0) return null;
  const roundedHours = Math.floor(totalHours);
  const days = Math.floor(roundedHours / 24);
  return days > 0 ? `${days}d ${roundedHours % 24}h (${roundedHours.toLocaleString()} hours)` : `${roundedHours} hours`;
}

function playerAvatarUrl(value: unknown, avatarId: unknown, webUrl: string): string {
  const canonicalAssetUrl = canonicalAvatarAssetUrl(avatarId);
  if (canonicalAssetUrl) return canonicalAssetUrl;
  const rawUrl = String(value ?? '').trim();
  if (/^https?:\/\//i.test(rawUrl)) return rawUrl;
  return `${webUrl.replace(/\/+$/, '')}${DEFAULT_PLAYER_AVATAR_PATH}`;
}

function formatDate(value: unknown): string | null {
  const date = new Date(String(value ?? ''));
  if (!Number.isFinite(date.getTime())) return null;
  return new Intl.DateTimeFormat('en-US', {
    year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC',
  }).format(date);
}

function globalKda(stats: unknown): string | null {
  if (!stats || typeof stats !== 'object') return null;
  const values = stats as Record<string, unknown>;
  const kills = number(values.kills);
  const deaths = number(values.deaths);
  const assists = number(values.assists);
  if (kills == null || deaths == null || assists == null) return null;
  const games = (number(values.wins) ?? 0) + (number(values.losses) ?? 0);
  if (kills + deaths + assists === 0 && games === 0) return null;
  return ((kills + assists / 2) / Math.max(deaths, 1)).toFixed(2);
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

function rankedField(label: string, tier: unknown, rank: unknown, points: unknown, wins: unknown, losses: unknown, leaves: unknown): APIEmbedField | null {
  const value = integer(tier) ?? 0;
  const games = (number(wins) ?? 0) + (number(losses) ?? 0);
  if (value <= 0 && games <= 0 && (number(points) ?? 0) <= 0) return null;
  const lines = [
    statLine('Rank', tierName(tier, rank)),
    statLine('TP', formatNumber(points)),
  ];
  const winRate = formatPercent(wins, losses);
  if (winRate) lines.push(statLine('Win rate', `${winRate} (${formatNumber(wins)}–${formatNumber(losses)})`));
  const leavesValue = number(leaves);
  if (leavesValue != null && leavesValue > 0) lines.push(statLine('Times deserted', formatNumber(leavesValue)));
  return { name: label, value: codeBlock(lines), inline: false };
}

function performanceField(player: Record<string, unknown>): APIEmbedField | null {
  const metrics = [
    ['DPM', player.avg_dpm], ['HPM', player.avg_hpm], ['MPM', player.avg_mpm], ['EGPM', player.avg_egpm],
  ]
    .map(([label, value]) => ({ label, value: number(value) }))
    .filter((metric): metric is { label: string; value: number } => metric.value != null && metric.value > 0);
  if (metrics.length === 0) return null;
  return {
    name: 'Ranked performance',
    value: codeBlock(metrics.map((metric) => statLine(metric.label, Math.round(metric.value).toLocaleString()))),
    inline: false,
  };
}

export function buildPlayerProfileMessage(
  response: PlayerProfileResponse,
  webUrl: string,
): DiscordMessagePayload {
  const player = response.player;
  const playerId = encodeURIComponent(String(player.id));
  const playerName = compact(player.name, 256) || 'Unknown player';
  const title = compact(player.title, 220);
  const heading = title ? `${playerName} (${title})`.slice(0, 256) : playerName;
  const fields: APIEmbedField[] = [];

  const record = formatPercent(player.wins, player.losses);
  const wins = number(player.wins) ?? 0;
  const losses = number(player.losses) ?? 0;
  const totalMatches = wins + losses;
  const kda = globalKda(response.globalStats);
  fields.push({
    name: 'General',
    value: codeBlock([
      statLine('Account ID', String(player.id)),
      statLine('Account level', formatNumber(player.level)),
      statLine('Total XP', formatNumber(player.total_xp)),
      statLine('Total matches', formatNumber(totalMatches)),
      statLine('Casual deserted', formatNumber(player.leaves)),
      statLine('Win rate', record ? `${record} (${formatNumber(player.wins)}–${formatNumber(player.losses)})` : '—'),
      ...(kda ? [statLine('Global KDA', kda)] : []),
    ]),
    inline: false,
  });

  const kbm = rankedField('Ranked KBM', player.kbm_tier, player.kbm_rank, player.kbm_points, player.kbm_wins, player.kbm_losses, player.kbm_leaves);
  const controller = rankedField('Ranked Controller', player.controller_tier, player.controller_rank, player.controller_points, player.controller_wins, player.controller_losses, player.controller_leaves);
  if (kbm) fields.push(kbm);
  if (controller) fields.push(controller);

  const otherLines = [
    statLine('Platform', compact(player.platform, 40) || 'Unknown'),
    statLine('Region', compact(player.region, 40) || 'Unknown'),
  ];
  const playtime = formatPlaytime(player.hours_played, player.minutes_played);
  if (playtime) otherLines.push(statLine('Playtime', playtime));
  const mastery = number(player.mastery_level);
  if (mastery != null && mastery > 0) otherLines.push(statLine('Mastery level', formatNumber(mastery)));
  const achievements = number(player.total_achievements);
  if (achievements != null && achievements > 0) otherLines.push(statLine('Achievements', formatNumber(achievements)));
  const createdAt = formatDate(player.created_datetime);
  if (createdAt) otherLines.push(statLine('Account created', createdAt));
  const lastLogin = formatDate(player.last_login_datetime);
  if (lastLogin) otherLines.push(statLine('Last login', lastLogin));
  const loadingFrame = compact(player.loading_frame, 80);
  if (loadingFrame) otherLines.push(statLine('Loading frame', loadingFrame));
  fields.push({ name: 'Other', value: codeBlock(otherLines), inline: false });

  const performance = performanceField(player);
  if (performance) fields.push(performance);

  const refreshedAt = new Date(String(response.profileRefresh?.refreshed_at ?? player.last_updated ?? ''));
  const timestamp = Number.isFinite(refreshedAt.getTime()) ? refreshedAt.toISOString() : undefined;
  const embed: APIEmbed = {
    color: accent,
    title: heading,
    url: `${webUrl}/players/${playerId}`,
    fields,
    thumbnail: { url: playerAvatarUrl(player.avatar_url, player.avatar_id, webUrl) },
    footer: { text: 'PaladinsCat' },
    timestamp,
  };

  return assertDiscordMessage({ embeds: [embed], allowedMentions: { parse: [] } });
}
