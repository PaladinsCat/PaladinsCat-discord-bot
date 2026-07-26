export class QueueFullError extends Error {}

export interface QueueSnapshot {
  active: number;
  queued: number;
  completed: number;
  failed: number;
  deduplicated: number;
  durationMs: { last: number; average: number; p95: number; max: number };
}

type Waiting<T> = {
  key: string;
  work: (signal: AbortSignal) => Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
};

export class BoundedWorkQueue<T> {
  private readonly waiting: Waiting<T>[] = [];
  private readonly inFlight = new Map<string, Promise<T>>();
  private active = 0;
  private completed = 0;
  private failed = 0;
  private deduplicated = 0;
  private readonly durations: number[] = [];

  constructor(
    private readonly concurrency: number,
    private readonly maxQueued: number,
    private readonly timeoutMs: number,
    private readonly workLabel = 'Render',
  ) {}

  add(key: string, work: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const existing = this.inFlight.get(key);
    if (existing) {
      this.deduplicated += 1;
      return existing;
    }
    if (this.waiting.length >= this.maxQueued) throw new QueueFullError('The image queue is busy. Try again shortly.');

    let resolve!: (value: T) => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
    this.inFlight.set(key, promise);
    this.waiting.push({ key, work, resolve, reject });
    this.drain();
    return promise;
  }

  snapshot(): QueueSnapshot {
    const sorted = [...this.durations].sort((left, right) => left - right);
    const total = this.durations.reduce((sum, duration) => sum + duration, 0);
    return {
      active: this.active,
      queued: this.waiting.length,
      completed: this.completed,
      failed: this.failed,
      deduplicated: this.deduplicated,
      durationMs: {
        last: this.durations.at(-1) ?? 0,
        average: this.durations.length ? Math.round(total / this.durations.length) : 0,
        p95: sorted.length ? sorted[Math.ceil(sorted.length * 0.95) - 1]! : 0,
        max: sorted.at(-1) ?? 0,
      },
    };
  }

  private drain() {
    while (this.active < this.concurrency && this.waiting.length > 0) {
      const item = this.waiting.shift()!;
      this.active += 1;
      const startedAt = performance.now();
      const controller = new AbortController();
      let timer: NodeJS.Timeout | undefined;
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          const error = new Error(`${this.workLabel} exceeded ${this.timeoutMs}ms`);
          controller.abort(error);
          reject(error);
        }, this.timeoutMs);
        timer.unref();
      });
      const work = Promise.resolve().then(() => item.work(controller.signal));
      Promise.race([work, timeout])
        .then((value) => {
          this.recordDuration(startedAt);
          this.completed += 1;
          item.resolve(value);
        })
        .catch((error) => {
          this.recordDuration(startedAt);
          this.failed += 1;
          item.reject(error);
        })
        .finally(() => {
          if (timer) clearTimeout(timer);
          this.active -= 1;
          this.inFlight.delete(item.key);
          this.drain();
        });
    }
  }

  private recordDuration(startedAt: number) {
    this.durations.push(Math.round(performance.now() - startedAt));
    if (this.durations.length > 100) this.durations.shift();
  }
}
