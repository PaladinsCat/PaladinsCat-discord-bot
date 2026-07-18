type Entry = { value: Buffer; expiresAt: number };

export class RenderCache {
  private readonly entries = new Map<string, Entry>();
  private bytes = 0;
  private hits = 0;
  private misses = 0;
  private evictions = 0;
  private expirations = 0;

  constructor(private readonly maxBytes: number, private readonly ttlMs: number) {}

  get(key: string): Buffer | undefined {
    const entry = this.entries.get(key);
    if (!entry) {
      this.misses += 1;
      return undefined;
    }
    if (entry.expiresAt <= Date.now()) {
      this.misses += 1;
      this.expirations += 1;
      this.delete(key, entry);
      return undefined;
    }
    this.hits += 1;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }

  set(key: string, value: Buffer) {
    if (this.maxBytes === 0 || value.byteLength > this.maxBytes) return;
    const current = this.entries.get(key);
    if (current) this.delete(key, current);
    while (this.bytes + value.byteLength > this.maxBytes && this.entries.size > 0) {
      const oldestKey = this.entries.keys().next().value as string;
      this.delete(oldestKey, this.entries.get(oldestKey)!);
      this.evictions += 1;
    }
    this.entries.set(key, { value, expiresAt: Date.now() + this.ttlMs });
    this.bytes += value.byteLength;
  }

  snapshot() {
    return {
      entries: this.entries.size,
      bytes: this.bytes,
      maxBytes: this.maxBytes,
      hits: this.hits,
      misses: this.misses,
      evictions: this.evictions,
      expirations: this.expirations,
    };
  }

  private delete(key: string, entry: Entry) {
    if (this.entries.delete(key)) this.bytes -= entry.value.byteLength;
  }
}
