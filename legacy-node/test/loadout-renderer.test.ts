import assert from 'node:assert/strict';
import test from 'node:test';
import { scaleCardDescription } from '../src/match-renderer.js';

test('scales Paladins card tokens cumulatively for the selected level', () => {
  assert.equal(
    scaleCardDescription('[Fireball] Reduce the Cooldown by {scale=0.4|0.4}s.', 4),
    'Reduce the Cooldown by 1.6s.',
  );
  assert.equal(
    scaleCardDescription('[Weapon] Increase Ammo by {1|1}.', 5),
    'Increase Ammo by 5.',
  );
});
