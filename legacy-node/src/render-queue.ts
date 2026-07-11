export class QueueFullError extends Error {}

export interface QueueSnapshot {
  active: number;
  queued: number;
  completed: number;
  failed: number;
  deduplicated: number;
}

type Waiting<T> = {
  key: string;
  work: () => Promise<T>;
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

  constructor(
    private readonly concurrency: number,
    private readonly maxQueued: number,
    private readonly timeoutMs: number,
  ) {}

  add(key: string, work: () => Promise<T>): Promise<T> {
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
    return { active: this.active, queued: this.waiting.length, completed: this.completed, failed: this.failed, deduplicated: this.deduplicated };
  }

  private drain() {
    while (this.active < this.concurrency && this.waiting.length > 0) {
      const item = this.waiting.shift()!;
      this.active += 1;
      let timer: NodeJS.Timeout;
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Render exceeded ${this.timeoutMs}ms`)), this.timeoutMs);
        timer.unref();
      });
      Promise.race([item.work(), timeout])
        .then((value) => { this.completed += 1; item.resolve(value); })
        .catch((error) => { this.failed += 1; item.reject(error); })
        .finally(() => {
          clearTimeout(timer);
          this.active -= 1;
          this.inFlight.delete(item.key);
          this.drain();
        });
    }
  }
}
