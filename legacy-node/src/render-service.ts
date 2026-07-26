import type { LoadoutRenderRecord, MatchRecord } from './types.js';
import { MatchRenderer } from './match-renderer.js';
import { BoundedWorkQueue } from './render-queue.js';
import { RenderCache } from './render-cache.js';

export class RenderService {
  private readonly queue: BoundedWorkQueue<Buffer>;
  private readonly lookupQueue: BoundedWorkQueue<MatchRecord>;
  private readonly cache: RenderCache;
  private readonly inFlightMatches = new Map<string, Promise<Buffer>>();
  private readonly renderAttemptTimeoutMs: number;
  private deduplicated = 0;
  private renderRetries = 0;
  private browserRecoveries = 0;

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
    // A healthy 2048x1152 scoreboard normally renders in 2-3 seconds. Abort a
    // poisoned page early enough to restart Chromium and retry inside the
    // command's existing total render budget.
    this.renderAttemptTimeoutMs = Math.max(1, Math.min(6000, Math.floor(options.timeoutMs * 0.4)));
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
    return this.queue.add(key, async (signal) => {
      const rendered = await this.renderWithRecovery((attemptSignal) => this.renderer.renderLoadout(record, attemptSignal), signal);
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
    return this.queue.add(key, async (signal) => {
      const rendered = await this.renderWithRecovery((attemptSignal) => this.renderer.render(record, attemptSignal), signal);
      this.cache.set(key, rendered);
      return rendered;
    });
  }

  private async renderWithRecovery(
    render: (signal: AbortSignal) => Promise<Buffer>,
    outerSignal: AbortSignal,
  ): Promise<Buffer> {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        return await this.renderAttempt(render, outerSignal);
      } catch (error) {
        await this.renderer.recycle();
        this.browserRecoveries += 1;
        if (outerSignal.aborted) throw outerSignal.reason ?? error;
        if (attempt === 1) throw error;
        this.renderRetries += 1;
      }
    }
    throw new Error('Render recovery exhausted');
  }

  private async renderAttempt(
    render: (signal: AbortSignal) => Promise<Buffer>,
    outerSignal: AbortSignal,
  ): Promise<Buffer> {
    outerSignal.throwIfAborted();
    const controller = new AbortController();
    const abortFromQueue = () => controller.abort(outerSignal.reason);
    outerSignal.addEventListener('abort', abortFromQueue, { once: true });
    let timer: NodeJS.Timeout | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        const error = new Error(`Render attempt exceeded ${this.renderAttemptTimeoutMs}ms`);
        controller.abort(error);
        reject(error);
      }, this.renderAttemptTimeoutMs);
      timer.unref();
    });
    try {
      return await Promise.race([render(controller.signal), timeout]);
    } finally {
      if (timer) clearTimeout(timer);
      outerSignal.removeEventListener('abort', abortFromQueue);
    }
  }

  warm() { return this.renderer.warm(); }
  close() { return this.renderer.close(); }
  snapshot() {
    return {
      lookup: this.lookupQueue.snapshot(),
      queue: this.queue.snapshot(),
      cache: this.cache.snapshot(),
      deduplicated: this.deduplicated,
      renderRetries: this.renderRetries,
      browserRecoveries: this.browserRecoveries,
      renderAttemptTimeoutMs: this.renderAttemptTimeoutMs,
    };
  }
}
