import assert from 'node:assert/strict';
import test from 'node:test';
import { syncDiscordCommands } from '../src/command-registration.js';

test('global registration clears stale guild command scopes without duplicating guild ids', async () => {
  const calls: Array<{ route: string; body: unknown[] }> = [];
  const rest = {
    async put(route: string, options: { body: unknown[] }) {
      calls.push({ route, body: options.body });
      return {};
    },
  };

  const result = await syncDiscordCommands(rest as never, 'app', undefined, ['guild-1', 'guild-2', 'guild-1'], [{ name: 'help' }]);

  assert.deepEqual(calls, [
    { route: '/applications/app/commands', body: [{ name: 'help' }] },
    { route: '/applications/app/guilds/guild-1/commands', body: [] },
    { route: '/applications/app/guilds/guild-2/commands', body: [] },
  ]);
  assert.deepEqual(result, {
    scope: 'global', registered: 1, clearedGuildScopes: 2, failedGuildScopes: 0,
  });
});

test('development registration updates only the selected guild scope', async () => {
  const calls: Array<{ route: string; body: unknown[] }> = [];
  const rest = {
    async put(route: string, options: { body: unknown[] }) {
      calls.push({ route, body: options.body });
      return {};
    },
  };

  const result = await syncDiscordCommands(rest as never, 'app', 'development-guild', ['other-guild'], [{ name: 'help' }]);

  assert.deepEqual(calls, [
    { route: '/applications/app/guilds/development-guild/commands', body: [{ name: 'help' }] },
  ]);
  assert.deepEqual(result, {
    scope: 'guild', registered: 1, clearedGuildScopes: 0, failedGuildScopes: 0,
  });
});
