import type { MatchRecord } from './types.js';
import { MatchRenderer } from './match-renderer.js';
import { BoundedWorkQueue } from './render-queue.js';
import { RenderCache } from './render-cache.js';

export class RenderService {
  private readonly queue: BoundedWorkQueue<Buffer>;
  private readonly cache: RenderCache;

  constructor(
    private readonly renderer: MatchRenderer,
    options: { concurrency: number; queueLimit: number; timeoutMs: number; cacheBytes: number; cacheTtlMs: number },
  ) {
    this.queue = new BoundedWorkQueue(options.concurrency, options.queueLimit, options.timeoutMs);
    this.cache = new RenderCache(options.cacheBytes, options.cacheTtlMs);
  }

  match(record: MatchRecord) {
    const key = `match:${record.match.match_id}:summary:v${this.renderer.templateVersion}`;
    const cached = this.cache.get(key);
    if (cached) return Promise.resolve(cached);
    return this.queue.add(key, async () => {
      const secondCheck = this.cache.get(key);
      if (secondCheck) return secondCheck;
      const rendered = await this.renderer.render(record);
      this.cache.set(key, rendered);
      return rendered;
    });
  }

  snapshot() { return { queue: this.queue.snapshot(), cache: this.cache.snapshot() }; }
}
