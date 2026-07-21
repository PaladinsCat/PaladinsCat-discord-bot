import fs from 'node:fs';
import path from 'node:path';
import puppeteer, { type Browser, type Page } from 'puppeteer-core';
import type { LoadoutRenderRecord, MatchPlayer, MatchRecord } from './types.js';
import { AssetCatalog } from './asset-catalog.js';

const WIDTH = 1280;
const HEIGHT = 720;
const MATCH_SCALE = 1.6;
const LOADOUT_SCALE = 1;
const TEMPLATE_VERSION = 13;
const LOADOUT_TEMPLATE_VERSION = 5;
const TIER_NAMES = ['Unranked', 'Bronze V', 'Bronze IV', 'Bronze III', 'Bronze II', 'Bronze I', 'Silver V', 'Silver IV', 'Silver III', 'Silver II', 'Silver I', 'Gold V', 'Gold IV', 'Gold III', 'Gold II', 'Gold I', 'Platinum V', 'Platinum IV', 'Platinum III', 'Platinum II', 'Platinum I', 'Diamond V', 'Diamond IV', 'Diamond III', 'Diamond II', 'Diamond I', 'Master', 'Grandmaster'];

const QUEUE_PRESENTATION: Record<number, { category: string; mode: string; ranked: boolean }> = {
  424: { category: 'Casual', mode: 'Siege', ranked: false },
  428: { category: 'Ranked', mode: 'Siege', ranked: false },
  437: { category: 'Casual', mode: 'Payload', ranked: false },
  451: { category: 'PvE', mode: 'Survival', ranked: false },
  452: { category: 'Casual', mode: 'Onslaught', ranked: false },
  469: { category: 'Casual', mode: 'Team Deathmatch', ranked: false },
  474: { category: 'Casual', mode: 'Battlegrounds Solo', ranked: false },
  475: { category: 'Casual', mode: 'Battlegrounds Duo', ranked: false },
  476: { category: 'Casual', mode: 'Battlegrounds Quad', ranked: false },
  486: { category: 'Ranked', mode: 'Siege', ranked: true },
};

function queuePresentation(queueId: number) {
  return QUEUE_PRESENTATION[queueId] ?? { category: 'Match', mode: 'Unknown mode', ranked: false };
}

export type MatchImageTheme = 'dark' | 'light';
export const DEFAULT_MATCH_IMAGE_THEME: MatchImageTheme = 'dark';

function xml(value: unknown) {
  return String(value ?? '').replace(/[<>&"']/g, (character) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&#39;' })[character]!);
}

function number(value: number | undefined) { return Math.round(Number(value ?? 0)).toLocaleString('en-US'); }
function score(value: number | null | undefined) { return value ?? '?'; }
function compact(value: number) { return Math.abs(value) >= 1000 ? `${(value / 1000).toFixed(1)}k` : number(value); }
function duration(seconds: number) { return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`; }

export function scaleCardDescription(description: string, level: number): string {
  const safeLevel = Math.max(1, Math.min(5, Math.floor(Number(level) || 1)));
  return description
    .replace(/^\s*\[[^\]]+]\s*/, '')
    .replace(/\{(?:scale=)?(-?[\d,]+(?:\.\d+)?)\|(-?[\d,]+(?:\.\d+)?)}/gi, (_match, baseText: string, stepText: string) => {
      const value = Number(baseText.replaceAll(',', '')) + Number(stepText.replaceAll(',', '')) * (safeLevel - 1);
      return Number(value.toFixed(2)).toLocaleString('en-US');
    })
    .replace(/\s+/g, ' ')
    .trim();
}
function utcTimestamp(value: string) {
  const trimmed = String(value ?? '').trim();
  const normalized = /^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?$/.test(trimmed)
    ? `${trimmed.replace(' ', 'T')}Z`
    : trimmed;
  const date = new Date(normalized);
  if (Number.isNaN(date.getTime())) return '—';
  const datePart = date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
  const timePart = date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', hour12: true, timeZone: 'UTC' });
  return `${datePart} · ${timePart} UTC`;
}
function damage(player: MatchPlayer) { return Number(player.damage_done_physical || player.damage_done_in_hand || 0); }
type TierSource = Partial<Pick<MatchPlayer, 'kbm_tier' | 'tier' | 'league_tier' | 'kbm_rank' | 'profile_snapshot'>>;

function baseTier(player: TierSource) {
  const value = Number(player.kbm_tier ?? player.tier ?? player.league_tier ?? 0);
  return Number.isFinite(value) ? Math.max(0, Math.min(27, Math.floor(value))) : 0;
}

/** Match the web scoreboard's display rule for the synthetic Grandmaster tier. */
export function matchPlayerDisplayTier(player: TierSource) {
  const value = baseTier(player);
  const rank = Number(player.kbm_rank ?? player.profile_snapshot?.kbm_rank ?? 0);
  return value === 26 && Number.isFinite(rank) && rank >= 1 && rank <= 100 ? 27 : value;
}
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
    const playwrightRoot = process.env.LOCALAPPDATA
      ? path.join(process.env.LOCALAPPDATA, 'ms-playwright')
      : null;
    if (playwrightRoot && fs.existsSync(playwrightRoot)) {
      const shells = fs.readdirSync(playwrightRoot)
        .filter((entry) => entry.startsWith('chromium_headless_shell-'))
        .sort((left, right) => right.localeCompare(left, undefined, { numeric: true }));
      for (const shell of shells) {
        const executable = path.join(playwrightRoot, shell, 'chrome-headless-shell-win64', 'chrome-headless-shell.exe');
        if (fs.existsSync(executable)) return executable;
      }
    }
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
  readonly loadoutTemplateVersion = LOADOUT_TEMPLATE_VERSION;
  readonly theme: MatchImageTheme;
  private readonly css: string;
  private readonly cheaterPatternUrl: string;
  private browserPromise: Promise<Browser> | null = null;
  private readonly idlePages = new Map<number, Page[]>();

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
    const cheaterPatternPath = path.join(path.dirname(templatePath), 'cheater-police-line.svg');
    if (!fs.existsSync(cheaterPatternPath)) throw new Error(`Cheater police-line asset was not found at ${cheaterPatternPath}.`);
    this.cheaterPatternUrl = `data:image/svg+xml,${encodeURIComponent(fs.readFileSync(cheaterPatternPath, 'utf8'))}`;
    this.chromiumPath = options.chromiumPath ?? defaultChromiumPath();
  }

  private readonly chromiumPath: string;

  async warm(): Promise<void> {
    const [matchPage, loadoutPage] = await Promise.all([
      this.acquirePage(MATCH_SCALE),
      this.acquirePage(LOADOUT_SCALE),
    ]);
    await Promise.all([
      this.releasePage(matchPage, MATCH_SCALE, true),
      this.releasePage(loadoutPage, LOADOUT_SCALE, true),
    ]);
  }

  async close(): Promise<void> {
    const pending = this.browserPromise;
    this.browserPromise = null;
    this.idlePages.clear();
    if (!pending) return;
    try {
      const browser = await pending;
      await browser.close();
    } catch {
      // A failed or already-disconnected browser has no remaining resources.
    }
  }

  async render(record: MatchRecord): Promise<Buffer> {
    return this.renderElement(this.document(record), '#scoreboard', 'Scoreboard markup did not render.', MATCH_SCALE);
  }

  async renderLoadout(record: LoadoutRenderRecord): Promise<Buffer> {
    return this.renderElement(this.loadoutDocument(record), '#loadout', 'Loadout markup did not render.', LOADOUT_SCALE);
  }

  private async renderElement(documentHtml: string, selector: string, missingMessage: string, scale: number): Promise<Buffer> {
    const page = await this.acquirePage(scale);
    let reusable = false;
    try {
      // The source prototype has an external font import. Waiting for the load
      // event lets a network-restricted render container hang on that request.
      await page.setContent(documentHtml, { waitUntil: 'domcontentloaded' });
      await page.evaluate(async () => {
        await document.fonts.ready;
        await Promise.all([...document.images].map(async (image) => {
          if (!image.complete) await new Promise<void>((resolve) => { image.addEventListener('load', () => resolve(), { once: true }); image.addEventListener('error', () => resolve(), { once: true }); });
          await image.decode().catch(() => undefined);
        }));
        await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      });
      const board = await page.$(selector);
      if (!board) throw new Error(missingMessage);
      const image = Buffer.from(await board.screenshot({ type: 'png', optimizeForSpeed: true }));
      reusable = true;
      return image;
    } finally {
      await this.releasePage(page, scale, reusable);
    }
  }

  private async acquirePage(scale: number): Promise<Page> {
    const pool = this.idlePages.get(scale) ?? [];
    let page = pool.pop();
    while (page?.isClosed()) page = pool.pop();
    if (page) return page;
    page = await (await this.browser()).newPage();
    await page.setViewport({ width: WIDTH, height: HEIGHT, deviceScaleFactor: scale });
    return page;
  }

  private async releasePage(page: Page, scale: number, reusable: boolean): Promise<void> {
    if (reusable && !page.isClosed() && this.browserPromise) {
      const pool = this.idlePages.get(scale) ?? [];
      pool.push(page);
      this.idlePages.set(scale, pool);
      return;
    }
    await page.close().catch(() => undefined);
  }

  private browser(): Promise<Browser> {
    if (this.browserPromise) return this.browserPromise;
    const pending = puppeteer.launch({
      executablePath: this.chromiumPath,
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage', '--disable-gpu', '--font-render-hinting=medium', '--allow-file-access-from-files'],
    });
    this.browserPromise = pending;
    void pending.then((browser) => {
      browser.once('disconnected', () => {
        if (this.browserPromise === pending) {
          this.browserPromise = null;
          this.idlePages.clear();
        }
      });
    }).catch(() => {
      if (this.browserPromise === pending) this.browserPromise = null;
    });
    return pending;
  }

  private document(record: MatchRecord) {
    const map = assetUrl(this.assets.mapImage(record.match.map));
    const background = map ? `#scoreboard::before{background-image:url('${map}')!important}` : '';
    return `<!doctype html><html><head><meta charset="utf-8"/><style>${this.css}\n${background}\nbody{--cheater-pattern:url("${this.cheaterPatternUrl}");min-height:720px;padding:0;background:transparent}.scoreboard{transform:none}.scoreboard-canvas{width:1280px;height:720px}.viewport{width:1280px;max-width:none}.prototype-note,.color-lab{display:none}</style></head><body data-theme="${this.theme}"><main class="viewport"><div class="scoreboard-canvas"><section class="scoreboard" id="scoreboard" aria-label="Paladins match scoreboard">${this.hero(record)}${this.columns()}<div class="players" id="team-one">${this.teamRows(record, 1)}</div>${this.summary(record, 1)}<div class="players" id="team-two">${this.teamRows(record, 2)}</div>${this.summary(record, 2)}</section></div></main></body></html>`;
  }

  private loadoutDocument(record: LoadoutRenderRecord) {
    const { player, loadout } = record;
    const championBanner = assetUrl(this.assets.championBanner(loadout.champion_name));
    const championIcon = assetUrl(this.assets.championIcon(loadout.champion_name));
    const brandIcon = assetUrl(this.assets.icon('paladinscat'));
    const cards = loadout.card_ids.slice(0, 5).map((cardId, index) => {
      const level = Math.max(1, Math.min(5, Math.floor(Number(loadout.card_levels[index]) || 1)));
      const card = this.assets.loadoutCard(Number(cardId));
      const name = card?.name ?? `Card ${cardId}`;
      const description = scaleCardDescription(card?.description || card?.shortDescription || 'Card details unavailable.', level);
      const artwork = assetUrl(card?.iconPath ?? null);
      const frame = this.assets.loadoutFrame(level);
      return `<article class="loadout-card level-${level}" aria-label="${xml(name)}, level ${level} ${xml(frame?.rarity ?? '')}">
        <img class="card-art" src="${artwork}" alt=""/>
        <img class="card-frame" src="${assetUrl(frame?.iconPath ?? null)}" alt=""/>
        <h2>${xml(name)}</h2>
        <p class="card-description">${xml(description)}</p>
        <span class="level-badge">${level}</span>
      </article>`;
    }).join('');
    const background = championBanner ? `url('${championBanner}')` : championIcon ? `url('${championIcon}')` : 'none';
    return `<!doctype html><html><head><meta charset="utf-8"><style>${this.css}
      *{box-sizing:border-box}html,body{margin:0;width:1280px;height:720px;min-height:720px;overflow:hidden;padding:0;background:transparent;color:var(--text);font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
      #loadout{position:relative;width:1280px;height:720px;overflow:hidden;border:1px solid rgba(111,130,153,.35);border-radius:20px;background:var(--bg)}
      #loadout::before{content:"";position:absolute;inset:0;background-image:${background};background-size:cover;background-position:center 24%;filter:saturate(1.15);opacity:.7;pointer-events:none}
      #loadout>*{z-index:1}
      .loadout-header{position:relative;height:238px;padding:28px 38px;display:flex;align-items:flex-start;background:rgba(5,9,15,.58);border-bottom:1px solid rgba(72,211,190,.22)}
      .loadout-header::before{content:"";position:absolute;inset:0;pointer-events:none;-webkit-backdrop-filter:blur(7px);backdrop-filter:blur(7px);-webkit-mask-image:linear-gradient(90deg,#000,transparent 20%,transparent 80%,#000);mask-image:linear-gradient(90deg,#000,transparent 20%,transparent 80%,#000)}
      .loadout-header>*{position:relative;z-index:1}.loadout-identity{min-width:0}.brand-line{display:flex;align-items:center;gap:12px;justify-content:flex-start}.brand-name{display:inline-flex;align-items:center;gap:11px;font-size:25px;line-height:1;font-weight:820;letter-spacing:-.025em}.brand-name img{width:32px;height:32px;border-radius:0;object-fit:contain}.loadout-status{height:23px;padding:0 9px;color:#bff7ee;border-color:rgba(55,214,192,.34);background:rgba(15,118,110,.25);font-size:10px}
      h1{margin:10px 0 0;padding-bottom:4px;max-width:760px;overflow:hidden;color:var(--text);font-size:54px;line-height:1.08;font-weight:760;letter-spacing:-.02em;text-overflow:ellipsis;white-space:nowrap;text-shadow:0 3px 5px rgba(0,0,0,.45)}
      .loadout-context{display:flex;align-items:center;gap:10px;margin-top:8px;color:#d3dce7;font-size:15px;line-height:1.1;font-weight:760;letter-spacing:.11em}.loadout-context .deck{color:#d3dce7}
      .loadout-context span+span::before{margin-right:9px}
      .cards{position:relative;z-index:2;display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:4px;padding:0 16px 18px;align-items:start;background:color-mix(in srgb,var(--bg) 80%,transparent);border-bottom:1px solid var(--line)}
      .cards::before{content:"";position:absolute;inset:0;pointer-events:none;-webkit-backdrop-filter:blur(3px);backdrop-filter:blur(3px)}.cards>*{position:relative;z-index:1}
      .loadout-card{position:relative;width:100%;aspect-ratio:316/480;filter:drop-shadow(0 3px 5px rgba(0,0,0,.45))}
      .card-art{position:absolute;z-index:1;left:6.5%;top:8.7%;width:87%;height:44%;object-fit:cover;background:#071014}
      .card-frame{position:absolute;z-index:2;inset:0;width:100%;height:100%;object-fit:fill;pointer-events:none}
      .loadout-card h2{position:absolute;z-index:3;left:9%;top:51.2%;width:82%;height:6.8%;margin:0;padding:0 5px;display:flex;align-items:center;justify-content:center;color:#fff;font-size:18px;line-height:1;text-align:center;text-shadow:0 2px 2px #111;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
      .card-description{position:absolute;z-index:3;left:9.5%;top:59.5%;width:81%;height:29%;margin:0;padding:4px 9px 0;display:flex;align-items:flex-start;justify-content:center;color:#303943;font-size:14px;line-height:1.25;font-weight:700;text-align:center;overflow:hidden}
      .level-badge{position:absolute;z-index:3;left:13.2%;top:92.7%;width:20%;aspect-ratio:1;transform:translate(-47%,-44%);display:flex;align-items:center;justify-content:center;padding:0;color:#f7fbff;font-size:27px;line-height:1;font-weight:820;font-variant-numeric:tabular-nums;text-align:center;text-shadow:0 2px 3px #10151d}
    </style></head><body data-theme="${this.theme}"><main id="loadout"><header class="loadout-header"><div class="loadout-identity"><div class="brand-line"><span class="brand-name"><img src="${brandIcon}" alt="">PaladinsCat</span><div class="status-tags"><span class="status-tag loadout-status">Loadout</span></div></div><h1>${xml(player.name)}</h1><div class="match-context loadout-context"><span>${xml(loadout.champion_name)}</span><span class="deck">${xml(loadout.loadout_name || 'Unnamed Loadout')}</span></div></div></header><section class="cards">${cards}</section></main></body></html>`;
  }

  private columns() {
    return `<div class="columns grid-row"><div>Party</div><div></div><div>Level</div><div>Player</div><div>Elo</div><div>Talent</div><div>Credits</div><div>K / D / A</div><div>OB. Time</div><div>Damage</div><div>Taken</div><div>Shielding</div><div>Healing</div></div>`;
  }

  private hero(record: MatchRecord) {
    const { match } = record;
    const mapName = match.map.replace(/^(?:(?:Ranked|Live|WIP)\s+)+/i, '').replace(/\bv\d+\b/ig, '').trim();
    const presentation = queuePresentation(match.queue_id);
    const ranked = presentation.ranked;
    const bans = [...(record.bans ?? [])].sort((a, b) => Number(a.ban_slot ?? 0) - Number(b.ban_slot ?? 0));
    const split = Math.ceil(bans.length / 2);
    const banSet = (entries: typeof bans) => entries.slice(0, 4).map((ban) => `<span class="ban-pick"><img src="${assetUrl(this.assets.championIcon(ban.champion_name))}" alt="${xml(ban.champion_name)}"/></span>`).join('');
    const averageTier = this.averageTier(record.players);
    const mapClass = mapName.length > 19 ? 'map-name long' : 'map-name';
    const banMarkup = ranked
      ? `<div class="score-bans left"><span class="ban-label">Bans</span><div class="ban-picks">${banSet(bans.slice(0, split))}</div></div>`
      : '';
    const rightBanMarkup = ranked
      ? `<div class="score-bans right"><span class="ban-label">Bans</span><div class="ban-picks">${banSet(bans.slice(split))}</div></div>`
      : '';
    const tierName = TIER_NAMES[averageTier] ?? 'Unranked';
    const tierMarkup = `<div class="tier-meta"${ranked ? '' : ' aria-hidden="true"'}><img src="${assetUrl(this.assets.rankIcon(averageTier))}" alt="${ranked ? xml(tierName) : ''}"/><div><div class="meta-value">${xml(tierName)}</div><div class="meta-label">Avg tier</div></div></div>`;
    const statusMarkup = [
      `<span class="status-tag ${ranked ? 'ranked' : 'casual'}">${ranked ? 'Ranked' : 'Casual'}</span>`,
      match.broken && !match.recovered ? '<span class="status-tag broken">Broken</span>' : '',
      match.recovered ? '<span class="status-tag recovered">Recovered</span>' : '',
      match.private ? '<span class="status-tag private">Private</span>' : '',
    ].join('');
    const contextMarkup = `<div class="match-context"><span>${xml(match.region || '—')}</span><span>${xml(presentation.mode)}</span></div>`;
    return `<header class="hero${ranked ? '' : ' casual'}"><div class="match-identity"><div class="brand-line"><span class="brand-name"><img src="${assetUrl(this.assets.icon('paladinscat'))}" alt=""/> PaladinsCat</span><div class="status-tags">${statusMarkup}</div></div><div class="map-line"><div class="${mapClass}" title="${xml(mapName)}">${xml(mapName)}</div></div>${contextMarkup}</div><div class="score${ranked ? '' : ' casual'}">${banMarkup}<span class="score-number team-one-score">${score(match.team1_score)}</span><span class="score-separator">/</span><span class="score-number team-two-score">${score(match.team2_score)}</span>${rightBanMarkup}</div><div class="match-meta${ranked ? '' : ' casual-meta'}">${tierMarkup}<time class="timestamp-meta" datetime="${xml(match.entry_datetime)}">${xml(utcTimestamp(match.entry_datetime))}</time><div class="duration-meta"><div class="meta-value">${duration(match.duration_seconds)}</div><div class="meta-label">Duration</div></div><div class="match-id-meta"><div class="meta-value">${xml(match.match_id)}</div><div class="meta-label">Match ID</div></div></div></header>`;
  }

  private teamRows(record: MatchRecord, team: 1 | 2) {
    const players = record.players.filter((player) => player.task_force === team).slice(0, 5);
    const facts = new Map((record.facts ?? []).map((fact) => [String(fact.player_id), fact]));
    const metrics = players.map((player) => this.metrics(player));
    const max = (key: keyof Metrics) => Math.max(0, ...metrics.map((values) => values[key]));
    return players.map((player, index) => {
      const values = metrics[index]!;
      const playerTier = matchPlayerDisplayTier(player);
      const fact = facts.get(String(player.player_id));
      const talent = fact?.talents?.[0];
      const talentIcon = talent ? this.assets.talentIcon(talent.talent_id, talent.champion_name || player.champion_name, talent.talent_name) : null;
      const peak = (key: keyof Metrics, requireValue = false) => values[key] === max(key) && (!requireValue || values[key] > 0) ? ' peak' : '';
      const level = Number(player.final_match_level ?? 0) || Number(player.account_level ?? 0);
      const cheater = Boolean(player.cheater);
      const suspicious = !cheater && Number(player.sus_count ?? 0) > 0;
      const verificationBadge = (player.verified ?? player.profile_snapshot?.verified)
        ? `<img class="verified-player-icon" src="${assetUrl(this.assets.icon('Verified_Player_Support_Icon', '.png'))}" alt="Verified PaladinsCat player"/>`
        : '';
      const moderationTag = cheater
        ? '<span class="player-status-tag cheater">CHEATER</span>'
        : suspicious ? '<span class="player-status-tag suspicious">SUS</span>' : '';
      return `<div class="player-row grid-row${cheater ? ' cheater-row' : ''}"><div class="champion-wrap"><img class="champion-icon" src="${assetUrl(this.assets.championIcon(player.champion_name))}" alt="${xml(player.champion_name)}"/>${party(player) ? `<span class="party-badge" title="Party ${party(player)}">${party(player)}</span>` : ''}</div><div class="rank"><img src="${assetUrl(this.assets.rankIcon(playerTier))}" alt="${xml(TIER_NAMES[playerTier] ?? 'Unranked')}"/></div><div class="level">${number(level)}</div><div class="player"><div class="player-name"><span class="player-name-text">${xml(player.player_name || 'PRIVATE')}</span>${verificationBadge}${moderationTag}</div><div class="player-sub">PID ${xml(player.player_id || 0)}</div></div><div class="player-elo">${player.queue_elo ? number(player.queue_elo) : '—'}</div><img class="talent-icon" src="${assetUrl(talentIcon)}" alt="${xml(talent?.talent_name ?? '')}"/><div class="metric credits${peak('credits')}"><img src="${assetUrl(this.assets.icon('Currency_Credits'))}" alt=""/>${number(values.credits)}</div><div class="metric kda">${player.kills} / ${player.deaths} / ${player.assists}</div><div class="metric obj${peak('objective')}">${number(values.objective)}</div><div class="metric damage${peak('damage')}">${number(values.damage)}</div><div class="metric taken${peak('taken')}">${number(values.taken)}</div><div class="metric shield${peak('shielding', true)}">${number(values.shielding)}</div><div class="metric heal${peak('healing', true)}">${number(values.healing)}</div></div>`;
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
    const tiers = players.map(baseTier).filter((value) => value >= 0);
    return tiers.length ? Math.floor(tiers.reduce((sum, value) => sum + value, 0) / tiers.length) : 0;
  }
}
