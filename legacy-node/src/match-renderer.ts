import sharp, { type OverlayOptions } from 'sharp';
import type { MatchPlayer, MatchRecord } from './types.js';
import { AssetCatalog } from './asset-catalog.js';

const WIDTH = 2048;
const HEIGHT = 1152;
const SCALE = 1.6;
const TEMPLATE_VERSION = 3;
const GRID_CENTERS = [46, 102, 161, 303, 444, 505, 589, 702, 796, 886, 994, 1102, 1210];

export type MatchImageTheme = 'dark' | 'light';
export const DEFAULT_MATCH_IMAGE_THEME: MatchImageTheme = 'dark';

function xml(value: unknown) {
  return String(value ?? '').replace(/[<>&"']/g, (char) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[char]!);
}

function number(value: number | undefined) { return Math.round(Number(value ?? 0)).toLocaleString('en-US'); }
function compact(value: number) { return Math.abs(value) >= 1000 ? `${(value / 1000).toFixed(1)}k` : number(value); }
function duration(seconds: number) { return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`; }
function damage(player: MatchPlayer) { return Number(player.damage_done_physical || player.damage_done_in_hand || 0); }
function tier(player: MatchPlayer) { const value = Number(player.tier ?? player.league_tier ?? 0); return Number.isFinite(value) ? Math.max(0, Math.min(27, Math.floor(value))) : 0; }
function party(player: MatchPlayer) { const value = Number(player.party ?? player.party_number ?? player.party_id ?? 0); return Number.isFinite(value) && value > 0 ? Math.floor(value) : null; }

type PlayerMetrics = { credits: number; objective: number; damage: number; taken: number; shielding: number; healing: number };

export class MatchRenderer {
  readonly templateVersion = TEMPLATE_VERSION;
  readonly theme: MatchImageTheme;

  constructor(private readonly assets: AssetCatalog, options: { theme?: MatchImageTheme } = {}) {
    this.theme = options.theme ?? DEFAULT_MATCH_IMAGE_THEME;
    sharp.concurrency(1);
    sharp.cache({ memory: 24, files: 0, items: 48 });
  }

  async render(record: MatchRecord): Promise<Buffer> {
    const teams = [1, 2].map((team) => record.players.filter((player) => player.task_force === team).slice(0, 5));
    const composites: OverlayOptions[] = [];
    const map = this.assets.mapImage(record.match.map);
    if (map) {
      try {
        const input = await sharp(map).resize(WIDTH, HEIGHT, { fit: 'cover' }).modulate({ saturation: 1.15 }).ensureAlpha(.7).png().toBuffer();
        composites.push({ input, left: 0, top: 0 });
      } catch { /* A missing map still produces a valid dark scoreboard. */ }
    }
    composites.push({ input: Buffer.from(this.svg(record, teams)), left: 0, top: 0 });

    const place = async (source: string | null, x: number, y: number, width: number, height: number, fit: 'cover' | 'contain' = 'cover') => {
      if (!source) return;
      try {
        const input = await sharp(source).resize(Math.round(width * SCALE), Math.round(height * SCALE), { fit }).png().toBuffer();
        composites.push({ input, left: Math.round(x * SCALE), top: Math.round(y * SCALE) });
      } catch { /* Text remains usable if an individual asset is unavailable. */ }
    };

    await place(this.assets.icon('paladinscat'), 28, 341, 22, 22, 'contain');
    await place(this.assets.rankIcon(this.averageTier(record.players)), 958, 357, 32, 32, 'contain');
    for (const [teamIndex, players] of teams.entries()) {
      const startY = teamIndex === 0 ? 26 : 416;
      for (const [rowIndex, player] of players.entries()) {
        const y = startY + rowIndex * 55;
        await place(this.assets.championIcon(player.champion_name), 20, y + 1.5, 52, 52);
        await place(this.assets.rankIcon(tier(player)), 80, y + 6.5, 44, 42, 'contain');
        await place(this.assets.icon('Currency_Credits'), 548, y + 20, 15, 15, 'contain');
      }
    }

    const sortedBans = [...(record.bans ?? [])].sort((a, b) => Number(a.ban_slot ?? 0) - Number(b.ban_slot ?? 0));
    const split = Math.ceil(sortedBans.length / 2);
    for (const [side, bans] of [sortedBans.slice(0, split), sortedBans.slice(split)].entries()) {
      const visible = bans.slice(0, 4);
      const startX = side === 0 ? 430 - visible.length * 28 : 850 - visible.length * 28;
      for (const [index, ban] of visible.entries()) await place(this.assets.championIcon(ban.champion_name), startX + index * 58, 352, 52, 52);
    }

    return sharp({ create: { width: WIDTH, height: HEIGHT, channels: 3, background: '#161618' } })
      .composite(composites)
      .png({ compressionLevel: 6, adaptiveFiltering: true })
      .toBuffer();
  }

  private svg(record: MatchRecord, teams: MatchPlayer[][]) {
    const match = record.match;
    const headers = ['PARTY', '', 'LEVEL', 'PLAYER', 'ELO', 'TALENT', 'CREDITS', 'K / D / A', 'OB. TIME', 'DAMAGE', 'TAKEN', 'SHIELDING', 'HEALING'];
    const header = headers.map((label, index) => `<text x="${GRID_CENTERS[index]}" y="15" class="column"${index === 3 ? ' text-anchor="start"' : ''}>${label}</text>`).join('');
    const playerRows = teams.map((players, teamIndex) => this.teamRows(players, teamIndex === 0 ? 1 : 2, teamIndex === 0 ? 26 : 416, match.winning_task_force)).join('');
    const avgTier = this.averageTier(record.players);
    const tierNames = ['Unranked','Bronze V','Bronze IV','Bronze III','Bronze II','Bronze I','Silver V','Silver IV','Silver III','Silver II','Silver I','Gold V','Gold IV','Gold III','Gold II','Gold I','Platinum V','Platinum IV','Platinum III','Platinum II','Platinum I','Diamond V','Diamond IV','Diamond III','Diamond II','Diamond I','Master','Grandmaster'];
    const mapName = match.map.replace(/^(?:(?:Ranked|Live)\s+)+/i, '');
    const ranked = match.queue_id === 486;
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 1280 720">
      <defs><filter id="glow"><feGaussianBlur stdDeviation="4" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>
      <style>
        text{font-family:Inter,"DejaVu Sans",Arial,sans-serif}.column{font-size:8.5px;font-weight:760;fill:#758397;text-anchor:middle;dominant-baseline:middle;letter-spacing:1px}
        .name{font-size:18px;font-weight:760;fill:#f4f7fb}.sub{font-size:12.5px;font-weight:550;fill:#8c99ab;letter-spacing:.4px}
        .value{font-size:21px;font-weight:720;fill:#f4f7fb;text-anchor:middle;dominant-baseline:middle}.small{font-size:18px}.summary{font-size:11.5px;font-weight:760;fill:#f4f7fb;text-anchor:middle;dominant-baseline:middle}
        .brand{font-size:13px;font-weight:760;fill:#f4f7fb}.map{font-size:23px;font-weight:780;fill:#f4f7fb}.queue{font-size:11px;font-weight:780;fill:#c2ccd8;letter-spacing:1.6px}
        .label{font-size:9px;font-weight:500;fill:#8c99ab;letter-spacing:1.1px}.meta{font-size:14px;font-weight:700;fill:#f4f7fb}.team{font-size:9px;font-weight:800;letter-spacing:1px}.result{font-size:8px;font-weight:760;letter-spacing:.7px}
      </style>
      <rect width="1280" height="720" fill="#161618" fill-opacity=".3"/>
      <rect width="1280" height="26" fill="#161618" fill-opacity=".8"/>${header}
      ${playerRows}
      <rect x="0" y="330" width="1280" height="86" fill="#161618" fill-opacity=".30" stroke="#37d6c0" stroke-opacity=".24"/>
      <text x="58" y="353" class="brand">PaladinsCat</text><text x="28" y="389" class="map">${xml(mapName)}</text>
      <text x="250" y="374" class="queue">${xml(match.region || '—')}</text><text x="250" y="386" class="queue">${ranked ? 'RANKED' : 'CASUAL'}</text><text x="250" y="398" class="queue">SIEGE</text>
      <text x="505" y="345" class="label" text-anchor="middle">BANS</text><text x="775" y="345" class="label" text-anchor="middle">BANS</text>
      <text x="602" y="374" font-size="41" font-weight="820" fill="#0c4493" text-anchor="middle">${match.team1_score}</text><text x="640" y="377" font-size="17" fill="#607086" text-anchor="middle">/</text><text x="678" y="384" font-size="41" font-weight="820" fill="#a52b2b" text-anchor="middle">${match.team2_score}</text>
      <text x="1010" y="359" class="label">AVG TIER</text><text x="1010" y="378" class="meta">${tierNames[avgTier] ?? 'Unranked'}</text>
      <text x="1080" y="359" class="label">DURATION</text><text x="1080" y="378" class="meta">${duration(match.duration_seconds)}</text>
      <text x="1252" y="359" class="label" text-anchor="end">MATCH ID</text><text x="1252" y="378" class="meta" text-anchor="end">${xml(match.match_id)}</text>
      <rect x=".5" y=".5" width="1279" height="719" rx="18" fill="none" stroke="#6f8299" stroke-opacity=".45"/>
    </svg>`;
  }

  private teamRows(players: MatchPlayer[], team: number, startY: number, winningTeam: number) {
    const rows = players.slice(0, 5);
    const metrics = rows.map((player): PlayerMetrics => ({ credits: Number(player.gold_earned ?? 0), objective: Number(player.objective_assists ?? 0), damage: damage(player), taken: Number(player.damage_taken ?? 0), shielding: Number(player.damage_mitigated ?? 0), healing: Number(player.healing ?? 0) }));
    const keys: Array<keyof PlayerMetrics> = ['credits','objective','damage','taken','shielding','healing'];
    const maxima = Object.fromEntries(keys.map((key) => [key, Math.max(0, ...metrics.map((row) => row[key]))])) as Record<keyof PlayerMetrics, number>;
    const colors = team === 1 ? ['#0b3d84','#072958'] : ['#952727','#631a1a'];
    const separators = [76,128,194,412,476,534,644,760,832,940,1048,1156];
    const rowSvg = Array.from({ length: 5 }, (_, index) => {
      const player = rows[index]; const y = startY + index * 55;
      if (!player) return `<rect y="${y}" width="1280" height="55" fill="${colors[index % 2]}" fill-opacity=".8"/>`;
      const values = metrics[index]!;
      const partyNumber = party(player);
      const underlines = ([['credits',589,'#f9c95f'],['objective',796,'#f4b974'],['damage',886,'#ff6675'],['taken',994,'#c94f60'],['shielding',1102,'#87a8ff'],['healing',1210,'#66e3a4']] as const)
        .filter(([key]) => values[key] > 0 && values[key] === maxima[key]).map(([,x,color]) => `<line x1="${x-28}" x2="${x+28}" y1="${y+49}" y2="${y+49}" stroke="${color}" stroke-width="2" stroke-opacity=".6" filter="url(#glow)"/>`).join('');
      return `<g><rect y="${y}" width="1280" height="55" fill="${colors[index % 2]}" fill-opacity=".8"/><line y1="${y+55}" y2="${y+55}" x2="1280" stroke="#94a3b8" stroke-opacity=".18"/>
        ${separators.map((x) => `<line x1="${x}" x2="${x}" y1="${y+5}" y2="${y+50}" stroke="#94a3b8" stroke-opacity=".2"/>`).join('')}
        ${partyNumber ? `<rect x="57" y="${y-2}" width="20" height="17" rx="8" fill="#0f766e" fill-opacity=".94"/><text x="67" y="${y+7}" font-size="9" font-weight="850" fill="#f5fffd" text-anchor="middle">${partyNumber}</text>` : ''}
        <text x="161" y="${y+28}" class="value" font-size="16.5">${number(player.final_match_level)}</text><text x="206" y="${y+22}" class="name">${xml(player.player_name || 'PRIVATE')}</text><text x="206" y="${y+42}" class="sub">PID ${xml(player.player_id || 0)}</text>
        <text x="444" y="${y+28}" class="value" font-size="16.5">${player.queue_elo ? number(player.queue_elo) : '—'}</text><text x="589" y="${y+28}" class="value">${number(values.credits)}</text><text x="702" y="${y+28}" class="value small">${player.kills} / ${player.deaths} / ${player.assists}</text>
        <text x="796" y="${y+28}" class="value">${number(values.objective)}</text><text x="886" y="${y+28}" class="value">${number(values.damage)}</text><text x="994" y="${y+28}" class="value">${number(values.taken)}</text><text x="1102" y="${y+28}" class="value">${number(values.shielding)}</text><text x="1210" y="${y+28}" class="value">${number(values.healing)}</text>${underlines}</g>`;
    }).join('');
    const summaryY = startY + 275;
    const divisor = Math.max(1, rows.length);
    const sum = (key: keyof PlayerMetrics) => metrics.reduce((total, row) => total + row[key], 0);
    const avgLevel = Math.round(rows.reduce((total, player) => total + Number(player.final_match_level ?? 0), 0) / divisor);
    const eloRows = rows.filter((player) => Number(player.queue_elo ?? 0) > 0);
    const avgElo = eloRows.length ? Math.round(eloRows.reduce((total, player) => total + Number(player.queue_elo), 0) / eloRows.length) : 0;
    const kda = `${rows.reduce((s,p)=>s+p.kills,0)} / ${rows.reduce((s,p)=>s+p.deaths,0)} / ${rows.reduce((s,p)=>s+p.assists,0)}`;
    const totals = [`AVG ${number(avgLevel)}`, avgElo ? `AVG ${number(avgElo)}` : 'AVG —', compact(sum('credits')), kda, compact(sum('objective')), compact(sum('damage')), compact(sum('taken')), compact(sum('shielding')), compact(sum('healing'))];
    const accent = team === 1 ? '#0c4493' : '#a52b2b';
    return `${rowSvg}<g><rect y="${summaryY}" width="1280" height="29" fill="#161618" fill-opacity=".8"/><circle cx="22" cy="${summaryY+14.5}" r="3" fill="${accent}"/><text x="31" y="${summaryY+17}" class="team" fill="${accent}">TEAM ${team}</text><text x="78" y="${summaryY+17}" class="result" fill="${accent}">${team === winningTeam ? 'WIN' : 'DEFEAT'}</text>${[161,444,589,702,796,886,994,1102,1210].map((x,index)=>`<text x="${x}" y="${summaryY+15}" class="summary">${totals[index]}</text>`).join('')}</g>`;
  }

  private averageTier(players: MatchPlayer[]) {
    const tiers = players.map(tier).filter((value) => value >= 0);
    return tiers.length ? Math.floor(tiers.reduce((sum, value) => sum + value, 0) / tiers.length) : 0;
  }
}
