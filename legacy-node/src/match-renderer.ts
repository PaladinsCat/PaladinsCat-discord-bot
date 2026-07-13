import fs from 'node:fs';
import path from 'node:path';
import puppeteer from 'puppeteer-core';
import type { MatchPlayer, MatchRecord } from './types.js';
import { AssetCatalog } from './asset-catalog.js';

const WIDTH = 1280;
const HEIGHT = 720;
const SCALE = 1.6;
const TEMPLATE_VERSION = 7;
const TIER_NAMES = ['Unranked', 'Bronze V', 'Bronze IV', 'Bronze III', 'Bronze II', 'Bronze I', 'Silver V', 'Silver IV', 'Silver III', 'Silver II', 'Silver I', 'Gold V', 'Gold IV', 'Gold III', 'Gold II', 'Gold I', 'Platinum V', 'Platinum IV', 'Platinum III', 'Platinum II', 'Platinum I', 'Diamond V', 'Diamond IV', 'Diamond III', 'Diamond II', 'Diamond I', 'Master', 'Grandmaster'];

export type MatchImageTheme = 'dark' | 'light';
export const DEFAULT_MATCH_IMAGE_THEME: MatchImageTheme = 'dark';

function xml(value: unknown) {
  return String(value ?? '').replace(/[<>&"']/g, (character) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&#39;' })[character]!);
}

function number(value: number | undefined) { return Math.round(Number(value ?? 0)).toLocaleString('en-US'); }
function compact(value: number) { return Math.abs(value) >= 1000 ? `${(value / 1000).toFixed(1)}k` : number(value); }
function duration(seconds: number) { return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`; }
function damage(player: MatchPlayer) { return Number(player.damage_done_physical || player.damage_done_in_hand || 0); }
function tier(player: MatchPlayer) { const value = Number(player.kbm_tier ?? player.tier ?? player.league_tier ?? 0); return Number.isFinite(value) ? Math.max(0, Math.min(27, Math.floor(value))) : 0; }
function party(player: MatchPlayer) { const value = Number(player.party ?? player.party_number ?? player.party_id ?? 0); return Number.isFinite(value) && value > 0 ? Math.floor(value) : null; }
const assetDataUrls = new Map<string, string>();

function assetUrl(source: string | null) {
  if (!source) return '';
  const cached = assetDataUrls.get(source);
  if (cached) return cached;
  const extension = path.extname(source).toLowerCase();
  const mime = extension === '.png' ? 'image/png' : extension === '.webp' ? 'image/webp' : extension === '.jpg' || extension === '.jpeg' ? 'image/jpeg' : 'image/avif';
  const value = `data:${mime};base64,${fs.readFileSync(source).toString('base64')}`;
  assetDataUrls.set(source, value);
  return value;
}

function defaultTemplatePath() {
  const configured = process.env.PALADINSCAT_SCOREBOARD_TEMPLATE;
  if (configured) return configured;
  const development = path.resolve(process.cwd(), '../../dev/prototypes/match-result-scoreboard.html');
  return fs.existsSync(development) ? development : path.resolve(process.cwd(), 'templates/match-result-scoreboard.html');
}

function defaultChromiumPath() {
  if (process.env.PALADINSCAT_CHROMIUM_PATH) return process.env.PALADINSCAT_CHROMIUM_PATH;
  if (process.platform === 'win32') {
    const edge = 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe';
    const chrome = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
    return fs.existsSync(edge) ? edge : chrome;
  }
  return '/usr/bin/chromium-browser';
}

type Metrics = { credits: number; objective: number; damage: number; taken: number; shielding: number; healing: number };

/**
 * Browser renderer for the Discord attachment. The CSS is extracted directly
 * from the development prototype; no visual rules are reimplemented in SVG.
 */
export class MatchRenderer {
  readonly templateVersion = TEMPLATE_VERSION;
  readonly theme: MatchImageTheme;
  private readonly css: string;

  constructor(
    private readonly assets: AssetCatalog,
    options: { theme?: MatchImageTheme; templatePath?: string; chromiumPath?: string } = {},
  ) {
    this.theme = options.theme ?? DEFAULT_MATCH_IMAGE_THEME;
    const templatePath = options.templatePath ?? defaultTemplatePath();
    const template = fs.readFileSync(templatePath, 'utf8');
    const match = template.match(/<style>([\s\S]*?)<\/style>/i);
    if (!match?.[1]) throw new Error(`Scoreboard prototype CSS was not found in ${templatePath}.`);
    // The prototype imports Inter from Google Fonts. Render containers must not
    // depend on an external font request, so the runtime image supplies the same
    // font through fontconfig and keeps every prototype rule otherwise unchanged.
    this.css = match[1].replace(/@import\s+url\(['"]https:\/\/fonts\.googleapis\.com\/[^'"]+['"]\);?/g, '');
    this.chromiumPath = options.chromiumPath ?? defaultChromiumPath();
  }

  private readonly chromiumPath: string;

  async render(record: MatchRecord): Promise<Buffer> {
    const browser = await this.launchBrowser();
    const page = await browser.newPage();
    try {
      await page.setViewport({ width: WIDTH, height: HEIGHT, deviceScaleFactor: SCALE });
      // The source prototype has an external font import. Waiting for the load
      // event lets a network-restricted render container hang on that request.
      await page.setContent(this.document(record), { waitUntil: 'domcontentloaded' });
      await page.evaluate(async () => {
        await document.fonts.ready;
        await Promise.all([...document.images].map(async (image) => {
          if (!image.complete) await new Promise<void>((resolve) => { image.addEventListener('load', () => resolve(), { once: true }); image.addEventListener('error', () => resolve(), { once: true }); });
          await image.decode().catch(() => undefined);
        }));
        await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      });
      const board = await page.$('#scoreboard');
      if (!board) throw new Error('Scoreboard markup did not render.');
      return Buffer.from(await board.screenshot({ type: 'png' }));
    } finally {
      await page.close();
      await browser.close();
    }
  }

  private launchBrowser() {
    return puppeteer.launch({
      executablePath: this.chromiumPath,
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage', '--disable-gpu', '--font-render-hinting=medium', '--allow-file-access-from-files'],
    });
  }

  private document(record: MatchRecord) {
    const map = assetUrl(this.assets.mapImage(record.match.map));
    const background = map ? `#scoreboard::before{background-image:url('${map}')!important}` : '';
    return `<!doctype html><html><head><meta charset="utf-8"/><style>${this.css}\n${background}\nbody{min-height:720px;padding:0;background:transparent}.scoreboard{transform:none}.scoreboard-canvas{width:1280px;height:720px}.viewport{width:1280px;max-width:none}.prototype-note{display:none}</style></head><body data-theme="${this.theme}"><main class="viewport"><div class="scoreboard-canvas"><section class="scoreboard" id="scoreboard" aria-label="Paladins match scoreboard">${this.hero(record)}${this.columns()}<div class="players" id="team-one">${this.teamRows(record, 1)}</div>${this.summary(record, 1)}<div class="players" id="team-two">${this.teamRows(record, 2)}</div>${this.summary(record, 2)}</section></div></main></body></html>`;
  }

  private columns() {
    return `<div class="columns grid-row"><div>Party</div><div></div><div>Level</div><div>Player</div><div>Elo</div><div>Talent</div><div>Credits</div><div>K / D / A</div><div>OB. Time</div><div>Damage</div><div>Taken</div><div>Shielding</div><div>Healing</div></div>`;
  }

  private hero(record: MatchRecord) {
    const { match } = record;
    const mapName = match.map.replace(/^(?:(?:Ranked|Live|WIP)\s+)+/i, '').replace(/\bv\d+\b/ig, '').trim();
    const ranked = match.queue_id === 486;
    const queue = ranked ? [match.region || '—', 'Ranked', 'Siege'] : [];
    const bans = [...(record.bans ?? [])].sort((a, b) => Number(a.ban_slot ?? 0) - Number(b.ban_slot ?? 0));
    const split = Math.ceil(bans.length / 2);
    const banSet = (entries: typeof bans) => entries.slice(0, 4).map((ban) => `<span class="ban-pick"><img src="${assetUrl(this.assets.championIcon(ban.champion_name))}" alt="${xml(ban.champion_name)}"/></span>`).join('');
    const averageTier = ranked ? this.averageTier(record.players) : null;
    const mapClass = mapName.length > 19 ? 'map-name long' : 'map-name';
    const banMarkup = ranked
      ? `<div class="score-bans left"><span class="ban-label">Bans</span><div class="ban-picks">${banSet(bans.slice(0, split))}</div></div>`
      : '';
    const rightBanMarkup = ranked
      ? `<div class="score-bans right"><span class="ban-label">Bans</span><div class="ban-picks">${banSet(bans.slice(split))}</div></div>`
      : '';
    const tierMarkup = averageTier === null
      ? ''
      : `<div class="tier-meta"><img src="${assetUrl(this.assets.rankIcon(averageTier))}" alt="${xml(TIER_NAMES[averageTier] ?? 'Unranked')}"/><div><div class="meta-value">${xml(TIER_NAMES[averageTier] ?? 'Unranked')}</div><div class="meta-label">Avg tier</div></div></div>`;
    const queueMarkup = ranked
      ? `<div class="queue">${queue.map((word) => `<span>${xml(word)}</span>`).join('')}</div>`
      : '';
    return `<header class="hero${ranked ? '' : ' casual'}"><div><div class="brand-line"><span class="brand-name"><img src="${assetUrl(this.assets.icon('paladinscat'))}" alt=""/> PaladinsCat</span>${queueMarkup}</div><div class="map-line"><div class="${mapClass}" title="${xml(mapName)}">${xml(mapName)}</div></div></div><div class="score${ranked ? '' : ' casual'}">${banMarkup}<span class="score-number team-one-score">${match.team1_score}</span><span class="score-separator">/</span><span class="score-number team-two-score">${match.team2_score}</span>${rightBanMarkup}</div><div class="match-meta${ranked ? '' : ' casual-meta'}">${tierMarkup}<div><div class="meta-value">${duration(match.duration_seconds)}</div><div class="meta-label">Duration</div></div><div><div class="meta-value">${xml(match.match_id)}</div><div class="meta-label">Match ID</div></div></div></header>`;
  }

  private teamRows(record: MatchRecord, team: 1 | 2) {
    const players = record.players.filter((player) => player.task_force === team).slice(0, 5);
    const facts = new Map((record.facts ?? []).map((fact) => [String(fact.player_id), fact]));
    const metrics = players.map((player) => this.metrics(player));
    const max = (key: keyof Metrics) => Math.max(0, ...metrics.map((values) => values[key]));
    return players.map((player, index) => {
      const values = metrics[index]!;
      const fact = facts.get(String(player.player_id));
      const talent = fact?.talents?.[0];
      const talentIcon = talent ? this.assets.talentIcon(talent.champion_name || player.champion_name, talent.talent_name) : null;
      const peak = (key: keyof Metrics, requireValue = false) => values[key] === max(key) && (!requireValue || values[key] > 0) ? ' peak' : '';
      const level = Number(player.final_match_level ?? 0) || Number(player.account_level ?? 0);
      return `<div class="player-row grid-row"><div class="champion-wrap"><img class="champion-icon" src="${assetUrl(this.assets.championIcon(player.champion_name))}" alt="${xml(player.champion_name)}"/>${party(player) ? `<span class="party-badge" title="Party ${party(player)}">${party(player)}</span>` : ''}</div><div class="rank"><img src="${assetUrl(this.assets.rankIcon(tier(player)))}" alt="${xml(TIER_NAMES[tier(player)] ?? 'Unranked')}"/></div><div class="level">${number(level)}</div><div class="player"><div class="player-name">${xml(player.player_name || 'PRIVATE')}</div><div class="player-sub">PID ${xml(player.player_id || 0)}</div></div><div class="player-elo">${player.queue_elo ? number(player.queue_elo) : '—'}</div><img class="talent-icon" src="${assetUrl(talentIcon)}" alt="${xml(talent?.talent_name ?? '')}"/><div class="metric credits${peak('credits')}"><img src="${assetUrl(this.assets.icon('Currency_Credits'))}" alt=""/>${number(values.credits)}</div><div class="metric kda">${player.kills} / ${player.deaths} / ${player.assists}</div><div class="metric obj${peak('objective')}">${number(values.objective)}</div><div class="metric damage${peak('damage')}">${number(values.damage)}</div><div class="metric taken${peak('taken')}">${number(values.taken)}</div><div class="metric shield${peak('shielding', true)}">${number(values.shielding)}</div><div class="metric heal${peak('healing', true)}">${number(values.healing)}</div></div>`;
    }).join('');
  }

  private summary(record: MatchRecord, team: 1 | 2) {
    const players = record.players.filter((player) => player.task_force === team).slice(0, 5);
    const metrics = players.map((player) => this.metrics(player));
    const sum = (key: keyof Metrics) => metrics.reduce((total, values) => total + values[key], 0);
    const divisor = Math.max(1, players.length);
    const level = Math.round(players.reduce((total, player) => total + (Number(player.final_match_level ?? 0) || Number(player.account_level ?? 0)), 0) / divisor);
    const elo = Math.round(players.reduce((total, player) => total + Number(player.queue_elo ?? 0), 0) / divisor);
    const kda = `${players.reduce((total, player) => total + player.kills, 0)} / ${players.reduce((total, player) => total + player.deaths, 0)} / ${players.reduce((total, player) => total + player.assists, 0)}`;
    const won = record.match.winning_task_force === team;
    const classes = team === 1 ? 'team-one' : 'team-two';
    return `<div class="team-bar ${classes} grid-row" id="team-${team === 1 ? 'one' : 'two'}-summary"><div class="team-heading"><div class="team-name">Team ${team} <span class="result">${won ? 'Win' : 'Defeat'}</span></div></div><div class="team-total level-total average-total"><span class="team-average-label">AVG</span>${number(level)}</div><div class="team-total elo-total average-total"><span class="team-average-label">AVG</span>${number(elo)}</div><div class="team-total credits-total"><img src="${assetUrl(this.assets.icon('Currency_Credits'))}" alt=""/>${compact(sum('credits'))}</div><div class="team-total kda-total">${kda}</div><div class="team-total objective-total">${compact(sum('objective'))}</div><div class="team-total damage-total">${compact(sum('damage'))}</div><div class="team-total taken-total">${compact(sum('taken'))}</div><div class="team-total shield-total">${compact(sum('shielding'))}</div><div class="team-total healing-total">${compact(sum('healing'))}</div></div>`;
  }

  private metrics(player: MatchPlayer): Metrics {
    return { credits: Number(player.gold_earned ?? 0), objective: Number(player.objective_assists ?? 0), damage: damage(player), taken: Number(player.damage_taken ?? 0), shielding: Number(player.damage_mitigated ?? 0), healing: Number(player.healing ?? 0) };
  }

  private averageTier(players: MatchPlayer[]) {
    const tiers = players.map(tier).filter((value) => value >= 0);
    return tiers.length ? Math.floor(tiers.reduce((sum, value) => sum + value, 0) / tiers.length) : 0;
  }
}
