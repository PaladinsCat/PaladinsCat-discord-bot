import fs from 'node:fs';
import path from 'node:path';

function normalized(value: string) {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export class AssetCatalog {
  private championFiles: string[] | null = null;
  private mapFiles: string[] | null = null;
  private rankFiles: string[] | null = null;

  constructor(private readonly root: string) {}

  championIcon(championName: string): string | null {
    const files = this.championFiles ??= this.loadChampionFiles();
    const wanted = normalized(`Champion ${championName} Icon`);
    return files.find((file) => normalized(path.parse(file).name) === wanted)
      ?? files.find((file) => normalized(path.parse(file).name).includes(normalized(championName)) && normalized(file).includes('icon'))
      ?? files.find((file) => normalized(path.parse(file).name) === normalized('Champion Generic Icon'))
      ?? null;
  }

  talentIcon(championName: string, talentName: string): string | null {
    const files = this.championFiles ??= this.loadChampionFiles();
    // Seris's published asset keeps its historical Soul Collector name.
    const assetName = championName === 'Seris' && talentName === 'Resuscitate'
      ? 'Seris Soul Collector'
      : `${championName} ${talentName}`;
    const wanted = normalized(`Talent ${assetName}`);
    const matches = files.filter((file) => normalized(path.parse(file).name) === wanted);
    // Sharp decodes the published talent AVIFs with an opaque black canvas on
    // Alpine/libvips. The matching PNGs preserve the intended transparency.
    return matches.find((file) => path.extname(file).toLowerCase() === '.png')
      ?? matches[0]
      ?? null;
  }

  mapImage(mapName: string): string | null {
    const files = this.mapFiles ??= this.loadFiles('maps');
    const wanted = normalized(mapName
      .replace(/^(?:(?:ranked|live|wip)\s+)+/i, '')
      .replace(/\bv\d+\b/ig, '')
      .trim());
    return files.find((file) => normalized(path.parse(file).name) === normalized(`Ranked ${wanted}`))
      ?? files.find((file) => normalized(path.parse(file).name).includes(wanted) && normalized(file).includes('ranked'))
      ?? files.find((file) => normalized(path.parse(file).name).includes(wanted))
      ?? null;
  }

  rankIcon(tier: number): string | null {
    const files = this.rankFiles ??= this.loadFiles('rank-tiers', true);
    if (tier <= 0) return files.find((file) => normalized(file).includes('rankiconqualifying')) ?? null;
    if (tier >= 27) return files.find((file) => normalized(file).includes('rankicongrandmaster')) ?? null;
    if (tier === 26) return files.find((file) => normalized(file).includes('rankiconmaster')) ?? null;
    const groups = ['Bronze', 'Silver', 'Gold', 'Platinum', 'Diamond'];
    const group = Math.floor((tier - 1) / 5);
    const division = 5 - ((tier - 1) % 5);
    const wanted = normalized(`RankIcon ${groups[group] ?? 'Bronze'} ${division}`);
    return files.find((file) => normalized(path.parse(file).name) === wanted) ?? null;
  }

  icon(name: string): string | null {
    const directory = path.join(this.root, 'icons');
    if (!fs.existsSync(directory)) return null;
    const wanted = normalized(name);
    const file = fs.readdirSync(directory).find((entry) => normalized(path.parse(entry).name) === wanted);
    return file ? path.join(directory, file) : null;
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
