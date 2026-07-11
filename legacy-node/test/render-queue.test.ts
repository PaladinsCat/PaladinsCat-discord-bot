import assert from 'node:assert/strict';
import test from 'node:test';
import { BoundedWorkQueue, QueueFullError } from '../src/render-queue.js';

test('deduplicates work with the same key', async () => {
  const queue = new BoundedWorkQueue<number>(1, 2, 1000);
  let runs = 0;
  const first = queue.add('same', async () => { runs += 1; await new Promise((resolve) => setTimeout(resolve, 20)); return 7; });
  const second = queue.add('same', async () => 8);
  assert.equal(await first, 7);
  assert.equal(await second, 7);
  assert.equal(runs, 1);
  assert.equal(queue.snapshot().deduplicated, 1);
});

test('rejects new work when the waiting queue is full', async () => {
  const queue = new BoundedWorkQueue<number>(1, 1, 1000);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const active = queue.add('active', async () => { await gate; return 1; });
  const waiting = queue.add('waiting', async () => 2);
  assert.throws(() => queue.add('overflow', async () => 3), QueueFullError);
  release();
  assert.deepEqual(await Promise.all([active, waiting]), [1, 2]);
});
