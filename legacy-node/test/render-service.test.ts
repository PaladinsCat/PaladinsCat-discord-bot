import assert from 'node:assert/strict';
import test from 'node:test';
import type { MatchRecord } from '../src/types.js';
import { RenderService } from '../src/render-service.js';
import type { MatchRenderer } from '../src/match-renderer.js';

function record(id: string): MatchRecord {
  return {
    match: {
      match_id: id,
      entry_datetime: '2026-07-18T00:00:00Z',
      queue_id: 486,
      duration_seconds: 600,
      region: 'NA',
      map: 'Ranked Stone Keep',
      team1_score: 4,
      team2_score: 2,
      winning_task_force: 1,
      broken: false,
      recovered: false,
      private: false,
    },
    players: [],
  };
}

test('checks the final image cache before loading match data', async () => {
  let renders = 0;
  let loads = 0;
  const renderer = {
    templateVersion: 99,
    render: async () => { renders += 1; return Buffer.from('image'); },
    warm: async () => undefined,
    close: async () => undefined,
  } as unknown as MatchRenderer;
  const service = new RenderService(renderer, {
    concurrency: 1,
    queueLimit: 2,
    timeoutMs: 1000,
    cacheBytes: 1024,
    cacheTtlMs: 1000,
  });
  const load = async () => { loads += 1; return record('123'); };

  assert.equal((await service.matchById('123', load)).toString(), 'image');
  assert.equal((await service.matchById('123', load)).toString(), 'image');
  assert.equal(loads, 1);
  assert.equal(renders, 1);
});

test('bounds and deduplicates slow match acquisition separately from the render timeout', async () => {
  let renders = 0;
  let loads = 0;
  const renderer = {
    templateVersion: 99,
    render: async () => { renders += 1; return Buffer.from('image'); },
    warm: async () => undefined,
    close: async () => undefined,
  } as unknown as MatchRenderer;
  const service = new RenderService(renderer, {
    concurrency: 1,
    queueLimit: 2,
    timeoutMs: 10,
    lookupConcurrency: 1,
    lookupQueueLimit: 2,
    lookupTimeoutMs: 100,
    cacheBytes: 1024,
    cacheTtlMs: 1000,
  });
  const load = async () => {
    loads += 1;
    await new Promise((resolve) => setTimeout(resolve, 30));
    return record('slow');
  };

  const [first, second] = await Promise.all([
    service.matchById('slow', load),
    service.matchById('slow', load),
  ]);

  assert.equal(first.toString(), 'image');
  assert.equal(second.toString(), 'image');
  assert.equal(loads, 1);
  assert.equal(renders, 1);
  assert.equal(service.snapshot().deduplicated, 1);
  assert.ok(service.snapshot().lookup.durationMs.last >= 25);
  assert.ok(service.snapshot().queue.durationMs.last < 10);
});
