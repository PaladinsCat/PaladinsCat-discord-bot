import fs from 'node:fs';
import path from 'node:path';
import type { LoadoutCardAsset, LoadoutFrameAsset } from './types.js';

function normalized(value: string) {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export class AssetCatalog {
  private championFiles: string[] | null = null;
  private mapFiles: string[] | null = null;
  private rankFiles: string[] | null = null;
  private iconFiles: string[] | null = null;
  private readonly championIcons = new Map<string, string | null>();
  private readonly championBanners = new Map<string, string | null>();
  private readonly talentIcons = new Map<string, string | null>();
  private readonly mapImages = new Map<string, string | null>();
  private readonly rankIcons = new Map<number, string | null>();
  private readonly icons = new Map<string, string | null>();
  private talentReferenceIcons: Map<number, string> | null = null;
  private cardReference: Map<number, LoadoutCardAsset> | null = null;
  private loadoutFrameReference: Map<number, LoadoutFrameAsset> | null = null;

  constructor(private readonly root: string) {}

  championIcon(championName: string): string | null {
    const key = normalized(championName);
    if (this.championIcons.has(key)) return this.championIcons.get(key)!;
    const files = this.championFiles ??= this.loadChampionFiles();
    const wanted = normalized(`Champion ${championName} Icon`);
    const result = files.find((file) => normalized(path.parse(file).name) === wanted)
      ?? files.find((file) => normalized(path.parse(file).name).includes(normalized(championName)) && normalized(file).includes('icon'))
      ?? files.find((file) => normalized(path.parse(file).name) === normalized('Champion Generic Icon'))
      ?? null;
    this.championIcons.set(key, result);
    return result;
  }

  championBanner(championName: string): string | null {
    const key = normalized(championName);
    if (this.championBanners.has(key)) return this.championBanners.get(key)!;
    const files = this.championFiles ??= this.loadChampionFiles();
    const wanted = normalized(`Banner ${championName}`);
    const matches = files.filter((file) => normalized(path.parse(file).name) === wanted);
    const result = matches.find((file) => path.extname(file).toLowerCase() === '.png')
      ?? matches[0]
      ?? this.championIcon(championName);
    this.championBanners.set(key, result);
    return result;
  }

  loadoutCard(cardId: number): LoadoutCardAsset | null {
    if (!this.cardReference) this.cardReference = this.loadCardReference();
    return this.cardReference.get(cardId) ?? null;
  }

  loadoutFrame(level: number): LoadoutFrameAsset | null {
    if (!this.loadoutFrameReference) this.loadoutFrameReference = this.loadLoadoutFrameReference();
    const safeLevel = Math.max(1, Math.min(5, Math.floor(Number(level) || 1)));
    return this.loadoutFrameReference.get(safeLevel) ?? null;
  }

  private loadLoadoutFrameReference(): Map<number, LoadoutFrameAsset> {
    const result = new Map<number, LoadoutFrameAsset>();
    const referencePath = path.resolve(this.root, '../data/paladins-loadout-frame-reference.json');
    if (!fs.existsSync(referencePath)) return result;
    try {
      const rows = JSON.parse(fs.readFileSync(referencePath, 'utf8')) as Array<Record<string, unknown>>;
      for (const row of rows) {
        const level = Number(row.level);
        const pngUrl = typeof row.pngUrl === 'string' ? row.pngUrl : '';
        const iconUrl = typeof row.iconUrl === 'string' ? row.iconUrl : '';
        if (!Number.isInteger(level) || level < 1 || level > 5) continue;
        const resolvePublicImage = (url: string) => url.startsWith('/images/')
          ? path.resolve(this.root, url.slice('/images/'.length))
          : null;
        const pngPath = resolvePublicImage(pngUrl);
        const iconPath = resolvePublicImage(iconUrl);
        const localPath = pngPath && fs.existsSync(pngPath)
          ? pngPath
          : iconPath && fs.existsSync(iconPath) ? iconPath : null;
        if (!localPath) continue;
        result.set(level, { level, rarity: String(row.rarity ?? `Level ${level}`), iconPath: localPath });
      }
    } catch (error) {
      console.warn(`[asset-catalog] Failed to load loadout frame reference ${referencePath}: ${error}`);
    }
    return result;
  }

  private loadCardReference(): Map<number, LoadoutCardAsset> {
    const result = new Map<number, LoadoutCardAsset>();
    const referencePath = path.resolve(this.root, '../data/paladins-card-reference.json');
    if (!fs.existsSync(referencePath)) return result;
    try {
      const rows = JSON.parse(fs.readFileSync(referencePath, 'utf8')) as Array<Record<string, unknown>>;
      const canonicalDescriptions = this.loadChampionCardDescriptions();
      for (const row of rows) {
        const id = Number(row.id);
        const iconUrl = typeof row.iconUrl === 'string' ? row.iconUrl : '';
        const name = String(row.name ?? `Card ${id}`);
        if (!Number.isInteger(id) || id <= 0) continue;
        const canonical = iconUrl.startsWith('/images/')
          ? path.resolve(this.root, iconUrl.slice('/images/'.length))
          : null;
        const png = canonical?.replace(/\.[^.]+$/, '.png') ?? null;
        const iconPath = png && fs.existsSync(png)
          ? png
          : canonical && fs.existsSync(canonical) ? canonical : null;
        result.set(id, {
          id,
          name,
          description: canonicalDescriptions.get(normalized(name)) ?? String(row.description ?? ''),
          shortDescription: String(row.shortDescription ?? ''),
          championId: Number(row.championId ?? 0),
          iconPath,
        });
      }
    } catch (error) {
      console.warn(`[asset-catalog] Failed to load card reference ${referencePath}: ${error}`);
    }
    return result;
  }

  private loadChampionCardDescriptions(): Map<string, string> {
    const descriptions = new Map<string, string>();
    const ambiguous = new Set<string>();
    const championDataPath = path.resolve(this.root, '../data/champion-data.json');
    if (!fs.existsSync(championDataPath)) return descriptions;
    try {
      const champions = JSON.parse(fs.readFileSync(championDataPath, 'utf8')) as Record<string, { loadouts?: Array<{ name?: unknown; description?: unknown }> }>;
      for (const champion of Object.values(champions)) {
        for (const card of champion.loadouts ?? []) {
          const name = typeof card.name === 'string' ? card.name : '';
          const description = typeof card.description === 'string' ? card.description.trim() : '';
          const key = normalized(name);
          if (!key || !description || ambiguous.has(key)) continue;
          const existing = descriptions.get(key);
          if (existing && existing !== description) {
            descriptions.delete(key);
            ambiguous.add(key);
          } else {
            descriptions.set(key, description);
          }
        }
      }
    } catch (error) {
      console.warn(`[asset-catalog] Failed to load champion card descriptions ${championDataPath}: ${error}`);
    }
    return descriptions;
  }

  talentIcon(talentId: number | null | undefined, championName: string, talentName: string): string | null {
    const key = `${Number(talentId) || 0}:${normalized(championName)}:${normalized(talentName)}`;
    if (this.talentIcons.has(key)) return this.talentIcons.get(key)!;
    const referenced = this.talentReferenceIcon(talentId);
    if (referenced) {
      this.talentIcons.set(key, referenced);
      return referenced;
    }
    const files = this.championFiles ??= this.loadChampionFiles();
    // Seris's published asset keeps its historical Soul Collector name.
    const assetName = championName === 'Seris' && talentName === 'Resuscitate'
      ? 'Seris Soul Collector'
      : `${championName} ${talentName}`;
    const wanted = normalized(`Talent ${assetName}`);
    const matches = files.filter((file) => normalized(path.parse(file).name) === wanted);
    // Prefer the matching PNG so both browser and historical renderer paths
    // preserve the intended transparent talent background.
    const result = matches.find((file) => path.extname(file).toLowerCase() === '.png')
      ?? matches[0]
      ?? null;
    this.talentIcons.set(key, result);
    return result;
  }

  private talentReferenceIcon(talentId: number | null | undefined): string | null {
    const id = Number(talentId);
    if (!Number.isInteger(id) || id <= 0) return null;
    if (!this.talentReferenceIcons) this.talentReferenceIcons = this.loadTalentReferenceIcons();
    return this.talentReferenceIcons.get(id) ?? null;
  }

  private loadTalentReferenceIcons(): Map<number, string> {
    const result = new Map<number, string>();
    const referencePath = path.resolve(this.root, '../data/paladins-talent-reference.json');
    if (!fs.existsSync(referencePath)) return result;
    try {
      const rows = JSON.parse(fs.readFileSync(referencePath, 'utf8')) as Array<{ id?: unknown; iconUrl?: unknown }>;
      for (const row of rows) {
        const id = Number(row.id);
        const iconUrl = typeof row.iconUrl === 'string' ? row.iconUrl : '';
        if (!Number.isInteger(id) || id <= 0 || !iconUrl.startsWith('/images/')) continue;
        const canonical = path.resolve(this.root, iconUrl.slice('/images/'.length));
        const png = canonical.replace(/\.[^.]+$/, '.png');
        if (fs.existsSync(png)) result.set(id, png);
        else if (fs.existsSync(canonical)) result.set(id, canonical);
      }
    } catch (error) {
      console.warn(`[asset-catalog] Failed to load talent reference ${referencePath}: ${error}`);
    }
    return result;
  }

  mapImage(mapName: string): string | null {
    const files = this.mapFiles ??= this.loadFiles('maps');
    const wanted = normalized(mapName
      .replace(/^(?:(?:ranked|live|wip)\s+)+/i, '')
      .replace(/\bv\d+\b/ig, '')
      .trim());
    if (this.mapImages.has(wanted)) return this.mapImages.get(wanted)!;
    const result = files.find((file) => normalized(path.parse(file).name) === normalized(`Ranked ${wanted}`))
      ?? files.find((file) => normalized(path.parse(file).name).includes(wanted) && normalized(file).includes('ranked'))
      ?? files.find((file) => normalized(path.parse(file).name).includes(wanted))
      ?? null;
    this.mapImages.set(wanted, result);
    return result;
  }

  rankIcon(tier: number): string | null {
    if (this.rankIcons.has(tier)) return this.rankIcons.get(tier)!;
    const files = this.rankFiles ??= this.loadFiles('rank-tiers', true);
    let result: string | null;
    if (tier <= 0) result = files.find((file) => normalized(file).includes('rankiconqualifying')) ?? null;
    else if (tier >= 27) result = files.find((file) => normalized(file).includes('rankicongrandmaster')) ?? null;
    else if (tier === 26) result = files.find((file) => normalized(file).includes('rankiconmaster')) ?? null;
    else {
      const groups = ['Bronze', 'Silver', 'Gold', 'Platinum', 'Diamond'];
      const group = Math.floor((tier - 1) / 5);
      const division = 5 - ((tier - 1) % 5);
      const wanted = normalized(`RankIcon ${groups[group] ?? 'Bronze'} ${division}`);
      result = files.find((file) => normalized(path.parse(file).name) === wanted) ?? null;
    }
    this.rankIcons.set(tier, result);
    return result;
  }

  icon(name: string, preferredExtension?: string): string | null {
    const key = `${normalized(name)}:${preferredExtension?.toLowerCase() ?? ''}`;
    if (this.icons.has(key)) return this.icons.get(key)!;
    const files = this.iconFiles ??= this.loadFiles('icons');
    const wanted = normalized(name);
    const matches = files.filter((file) => normalized(path.parse(file).name) === wanted);
    const preferred = preferredExtension?.toLowerCase();
    const result = (preferred ? matches.find((file) => path.extname(file).toLowerCase() === preferred) : null)
      ?? matches[0];
    this.icons.set(key, result ?? null);
    return result ?? null;
  }

  private loadChampionFiles() {
    return this.loadFiles('champions');
  }

  private loadFiles(directoryName: string, recursive = false): string[] {
    const directory = path.join(this.root, directoryName);
    if (!fs.existsSync(directory)) return [];
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
      const target = path.join(directory, entry.name);
      if (entry.isDirectory()) return recursive ? this.loadNestedFiles(target) : [];
      return /\.(avif|png|webp|jpe?g)$/i.test(entry.name) ? [target] : [];
    });
  }

  private loadNestedFiles(directory: string): string[] {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
      const target = path.join(directory, entry.name);
      if (entry.isDirectory()) return this.loadNestedFiles(target);
      return /\.(avif|png|webp|jpe?g)$/i.test(entry.name) ? [target] : [];
    });
  }
}
