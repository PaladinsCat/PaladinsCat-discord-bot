import type { LoadoutRenderRecord, MatchRecord } from './types.js';
import { MatchRenderer } from './match-renderer.js';
import { BoundedWorkQueue } from './render-queue.js';
import { RenderCache } from './render-cache.js';

export class RenderService {
  private readonly queue: BoundedWorkQueue<Buffer>;
  private readonly lookupQueue: BoundedWorkQueue<MatchRecord>;
  private readonly cache: RenderCache;
  private readonly inFlightMatches = new Map<string, Promise<Buffer>>();
  private deduplicated = 0;

  constructor(
    private readonly renderer: MatchRenderer,
    options: {
      concurrency: number;
      queueLimit: number;
      timeoutMs: number;
      cacheBytes: number;
      cacheTtlMs: number;
      lookupConcurrency?: number;
      lookupQueueLimit?: number;
      lookupTimeoutMs?: number;
    },
  ) {
    this.queue = new BoundedWorkQueue(options.concurrency, options.queueLimit, options.timeoutMs);
    this.lookupQueue = new BoundedWorkQueue(
      options.lookupConcurrency ?? 2,
      options.lookupQueueLimit ?? options.queueLimit,
      options.lookupTimeoutMs ?? 125000,
      'Match lookup',
    );
    this.cache = new RenderCache(options.cacheBytes, options.cacheTtlMs);
  }

  match(record: MatchRecord) {
    return this.renderRecord(record);
  }

  loadout(record: LoadoutRenderRecord) {
    const updatedAt = record.loadout.updated_at || record.loadout.fetched_at || 'unknown';
    const key = `loadout:${record.player.id}:${record.loadout.id}:${updatedAt}:v${this.renderer.loadoutTemplateVersion}`;
    const cached = this.cache.get(key);
    if (cached) return Promise.resolve(cached);
    return this.queue.add(key, async () => {
      const rendered = await this.renderer.renderLoadout(record);
      this.cache.set(key, rendered);
      return rendered;
    });
  }

  matchById(matchId: string, load: () => Promise<MatchRecord>) {
    const key = `match:${matchId}:summary:v${this.renderer.templateVersion}`;
    const cached = this.cache.get(key);
    if (cached) return Promise.resolve(cached);
    const existing = this.inFlightMatches.get(key);
    if (existing) {
      this.deduplicated += 1;
      return existing;
    }
    const pending = this.lookupQueue.add(key, load)
      .then((record) => this.renderRecord(record, key, true))
      .finally(() => this.inFlightMatches.delete(key));
    this.inFlightMatches.set(key, pending);
    return pending;
  }

  private renderRecord(
    record: MatchRecord,
    key = `match:${record.match.match_id}:summary:v${this.renderer.templateVersion}`,
    cacheAlreadyChecked = false,
  ) {
    if (!cacheAlreadyChecked) {
      const cached = this.cache.get(key);
      if (cached) return Promise.resolve(cached);
    }
    return this.queue.add(key, async () => {
      const rendered = await this.renderer.render(record);
      this.cache.set(key, rendered);
      return rendered;
    });
  }

  warm() { return this.renderer.warm(); }
  close() { return this.renderer.close(); }
  snapshot() {
    return {
      lookup: this.lookupQueue.snapshot(),
      queue: this.queue.snapshot(),
      cache: this.cache.snapshot(),
      deduplicated: this.deduplicated,
    };
  }
}
