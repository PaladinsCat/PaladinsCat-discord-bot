import assert from 'node:assert/strict';
import test from 'node:test';
import { RenderCache } from '../src/render-cache.js';

test('evicts the least recently used buffers by byte budget', () => {
  const cache = new RenderCache(6, 10000);
  cache.set('a', Buffer.from('aaa'));
  cache.set('b', Buffer.from('bbb'));
  assert.equal(cache.get('a')?.toString(), 'aaa');
  cache.set('c', Buffer.from('ccc'));
  assert.equal(cache.get('b'), undefined);
  assert.equal(cache.get('a')?.toString(), 'aaa');
  assert.equal(cache.get('c')?.toString(), 'ccc');
  assert.deepEqual(cache.snapshot(), {
    entries: 2,
    bytes: 6,
    maxBytes: 6,
    hits: 3,
    misses: 1,
    evictions: 1,
    expirations: 0,
  });
});
