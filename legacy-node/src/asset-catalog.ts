import fs from 'node:fs';
import path from 'node:path';

function normalized(value: string) {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export class AssetCatalog {
  private championFiles: string[] | null = null;
  private mapFiles: string[] | null = null;
  private rankFiles: string[] | null = null;
  private iconFiles: string[] | null = null;
  private readonly championIcons = new Map<string, string | null>();
  private readonly talentIcons = new Map<string, string | null>();
  private readonly mapImages = new Map<string, string | null>();
  private readonly rankIcons = new Map<number, string | null>();
  private readonly icons = new Map<string, string | null>();

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

  talentIcon(championName: string, talentName: string): string | null {
    const key = `${normalized(championName)}:${normalized(talentName)}`;
    if (this.talentIcons.has(key)) return this.talentIcons.get(key)!;
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
