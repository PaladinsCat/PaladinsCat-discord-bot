import fs from 'node:fs';
import path from 'node:path';

function normalized(value: string) {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export class AssetCatalog {
  private championFiles: string[] | null = null;

  constructor(private readonly root: string) {}

  championIcon(championName: string): string | null {
    const files = this.championFiles ??= this.loadChampionFiles();
    const wanted = normalized(`Champion ${championName} Icon`);
    return files.find((file) => normalized(path.parse(file).name) === wanted)
      ?? files.find((file) => normalized(path.parse(file).name).includes(normalized(championName)) && normalized(file).includes('icon'))
      ?? files.find((file) => normalized(path.parse(file).name) === normalized('Champion Generic Icon'))
      ?? null;
  }

  private loadChampionFiles() {
    const directory = path.join(this.root, 'champions');
    if (!fs.existsSync(directory)) return [];
    return fs.readdirSync(directory)
      .filter((file) => /\.(avif|png|webp|jpe?g)$/i.test(file))
      .map((file) => path.join(directory, file));
  }
}
