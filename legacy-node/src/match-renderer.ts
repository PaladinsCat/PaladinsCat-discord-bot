import sharp, { type OverlayOptions } from 'sharp';
import type { MatchPlayer, MatchRecord } from './types.js';
import { AssetCatalog } from './asset-catalog.js';

const WIDTH = 1280;
const HEIGHT = 720;
const TEMPLATE_VERSION = 1;

function xml(value: unknown) {
  return String(value ?? '').replace(/[<>&"']/g, (char) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[char]!);
}

function compact(value: number | undefined) {
  const number = Number(value ?? 0);
  return Math.abs(number) >= 1000 ? `${(number / 1000).toFixed(number >= 100000 ? 0 : 1)}k` : String(Math.round(number));
}

function duration(seconds: number) { return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`; }

export class MatchRenderer {
  readonly templateVersion = TEMPLATE_VERSION;

  constructor(private readonly assets: AssetCatalog) {
    sharp.concurrency(1);
    sharp.cache({ memory: 16, files: 0, items: 32 });
  }

  async render(record: MatchRecord): Promise<Buffer> {
    const teams = [1, 2].map((team) => record.players.filter((player) => player.task_force === team).slice(0, 5));
    const svg = this.svg(record, teams);
    const composites: OverlayOptions[] = [{ input: Buffer.from(svg), left: 0, top: 0 }];

    for (const [teamIndex, players] of teams.entries()) {
      for (const [rowIndex, player] of players.entries()) {
        const icon = this.assets.championIcon(player.champion_name);
        if (!icon) continue;
        try {
          const input = await sharp(icon).resize(60, 60, { fit: 'cover' }).png().toBuffer();
          composites.push({ input, left: teamIndex === 0 ? 42 : 662, top: 180 + rowIndex * 92 });
        } catch {
          // Text-only rows remain usable if one source asset is missing/corrupt.
        }
      }
    }

    return sharp({ create: { width: WIDTH, height: HEIGHT, channels: 3, background: '#10141b' } })
      .composite(composites)
      .jpeg({ quality: 84, mozjpeg: true, chromaSubsampling: '4:4:4' })
      .toBuffer();
  }

  private svg(record: MatchRecord, teams: MatchPlayer[][]) {
    const match = record.match;
    const rows = teams.flatMap((players, teamIndex) => players.map((player, rowIndex) => {
      const x = teamIndex === 0 ? 28 : 648;
      const y = 168 + rowIndex * 92;
      const won = player.task_force === match.winning_task_force;
      const source = player.source === 'recovered' ? 'REC' : player.private_slot ? 'PRIVATE' : '';
      return `<g>
        <rect x="${x}" y="${y}" width="604" height="80" rx="12" fill="${won ? '#173b32' : '#201f2b'}" stroke="${won ? '#2dd4a3' : '#34394a'}"/>
        <text x="${x + 80}" y="${y + 27}" class="name">${xml(player.player_name || 'PRIVATE')}</text>
        <text x="${x + 80}" y="${y + 54}" class="sub">${xml(player.champion_name)}${source ? ` · ${source}` : ''}</text>
        <text x="${x + 320}" y="${y + 31}" class="stat">${player.kills}/${player.deaths}/${player.assists}</text>
        <text x="${x + 320}" y="${y + 57}" class="label">K / D / A</text>
        <text x="${x + 420}" y="${y + 31}" class="stat">${compact(player.damage_done_physical || player.damage_done_in_hand)}</text>
        <text x="${x + 420}" y="${y + 57}" class="label">DMG</text>
        <text x="${x + 505}" y="${y + 31}" class="stat">${compact(player.healing)}</text>
        <text x="${x + 505}" y="${y + 57}" class="label">HEAL</text>
      </g>`;
    })).join('');
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}">
      <style>.title{font:700 31px sans-serif;fill:#f5f7fa}.meta{font:16px sans-serif;fill:#9ba4b5}.score{font:700 38px sans-serif;fill:#53dfc0}.team{font:700 18px sans-serif;fill:#dce2eb}.name{font:700 16px sans-serif;fill:#f3f5f8}.sub{font:13px sans-serif;fill:#9ba4b5}.stat{font:700 17px sans-serif;fill:#f3f5f8}.label{font:11px sans-serif;fill:#778195}</style>
      <rect width="1280" height="720" fill="#10141b"/><rect x="0" y="0" width="1280" height="8" fill="#2dd4a3"/>
      <text x="30" y="51" class="title">${xml(match.map || 'Paladins Match')}</text>
      <text x="30" y="79" class="meta">Match ${xml(match.match_id)} · Queue ${match.queue_id} · ${xml(match.region)} · ${duration(match.duration_seconds)}</text>
      <text x="640" y="58" text-anchor="middle" class="score">${match.team1_score} — ${match.team2_score}</text>
      <text x="28" y="145" class="team">TEAM 1${match.winning_task_force === 1 ? ' · WINNER' : ''}</text>
      <text x="648" y="145" class="team">TEAM 2${match.winning_task_force === 2 ? ' · WINNER' : ''}</text>
      ${rows}
      <text x="30" y="700" class="meta">PaladinsCat · ${match.broken ? 'partial recovery' : 'authoritative match'}${match.private ? ' · private account slot' : ''}</text>
    </svg>`;
  }
}
