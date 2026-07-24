import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatInputCommandInteraction } from 'discord.js';
import { PaladinsCatApi } from '../src/api-client.js';
import { CommandHandler } from '../src/commands.js';
import type { RenderService } from '../src/render-service.js';

type MockInteraction = ChatInputCommandInteraction & {
  replies: unknown[];
};

function interaction(
  commandName: string,
  discordUserId: string,
  values: Record<string, string | undefined> = {},
): MockInteraction {
  const replies: unknown[] = [];
  const mock = {
    commandName,
    user: { id: discordUserId },
    options: {
      getString(name: string, required?: boolean) {
        const value = values[name] ?? null;
        if (required && value == null) throw new Error(`Missing required option ${name}`);
        return value;
      },
    },
    deferred: false,
    replied: false,
    replies,
    async deferReply() {
      mock.deferred = true;
    },
    async editReply(payload: unknown) {
      replies.push(payload);
      mock.replied = true;
      return payload;
    },
    async reply(payload: unknown) {
      replies.push(payload);
      mock.replied = true;
      return payload;
    },
  };
  return mock as unknown as MockInteraction;
}

function backendFetch(requests: string[]): typeof fetch {
  const saved = new Map<string, { id: string; name: string }>();
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input));
    requests.push(`${init?.method ?? 'GET'} ${url.pathname}${url.search}`);

    if (url.pathname === '/players/discord/saved-player' && init?.method === 'PUT') {
      const body = JSON.parse(String(init.body)) as { discordUserId: string; playerId: string };
      const player = { id: body.playerId, name: 'NabiCookTV' };
      saved.set(body.discordUserId, player);
      return Response.json({ player });
    }
    if (url.pathname === '/players/discord/saved-player') {
      const player = saved.get(url.searchParams.get('discordUserId') ?? '');
      return player
        ? Response.json({ player })
        : Response.json({
          error: {
            code: 'NO_SAVED_PLAYER',
            message: 'No saved player is linked to this Discord account',
          },
        }, { status: 404 });
    }
    if (url.pathname === '/players/discord') {
      const inputPlayer = url.searchParams.get('player');
      const player = inputPlayer === 'NabiCookTV' || inputPlayer === '716515038'
        ? { id: '716515038', name: 'NabiCookTV', wins: 10, losses: 5 }
        : { id: '99', name: String(inputPlayer), wins: 1, losses: 1 };
      return Response.json({ player, globalStats: null });
    }
    if (url.pathname === '/players/716515038/matches') {
      return Response.json([]);
    }
    if (url.pathname === '/live/players/716515038') {
      return Response.json({ player_id: '716515038', match: null, players: [] });
    }
    if (url.pathname === '/players/716515038/loadouts') {
      return Response.json({
        loadouts: [{
          id: '1',
          deck_id: '1',
          deck_key: '716515038:1',
          champion_id: 2205,
          champion_name: 'Androxus',
          loadout_name: 'Default',
          card_ids: [1, 2, 3, 4, 5],
          card_levels: [5, 4, 3, 2, 1],
          talent_id: 9,
          fetched_at: '2026-07-24T00:00:00.000Z',
          updated_at: '2026-07-24T00:00:00.000Z',
        }],
        freshness: {
          ttl_seconds: 86400,
          refreshed_at: '2026-07-24T00:00:00.000Z',
          expires_at: '2026-07-25T00:00:00.000Z',
          remaining_seconds: 86400,
          expired: false,
          manual_refresh_available_at: null,
          manual_refresh_remaining_seconds: 0,
        },
        refreshed: false,
      });
    }
    return Response.json({ error: { message: 'Unexpected test request' } }, { status: 500 });
  }) as typeof fetch;
}

test('save then every player command resolves the Discord-linked default end to end', async () => {
  const requests: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, { fetchImpl: backendFetch(requests) });
  const handler = new CommandHandler(api, {} as RenderService, 'https://paladinscat.com');
  const userId = 'test-user-1';

  const save = interaction('save', userId, { player: 'NabiCookTV' });
  await handler.handle(save);
  assert.match(
    (save.replies[0] as { content: string }).content,
    /Saved \*\*NabiCookTV\*\* \(ID: `716515038`\)/,
  );

  const profile = interaction('profile', userId);
  await handler.handle(profile);
  const embed = (profile.replies[0] as { embeds: Array<{ title: string; url: string }> }).embeds[0];
  assert.ok(embed);
  assert.equal(embed.title, 'NabiCookTV');
  assert.equal(embed.url, 'https://paladinscat.com/players/716515038');

  const playerAlias = interaction('player', userId);
  await handler.handle(playerAlias);
  const history = interaction('history', userId);
  await handler.handle(history);
  const current = interaction('current', userId);
  await handler.handle(current);
  const loadout = interaction('loadout', userId, { champion: 'Androxus' });
  await handler.handle(loadout);

  assert.deepEqual(requests, [
    'GET /players/discord?player=NabiCookTV',
    'PUT /players/discord/saved-player',
    'GET /players/discord/saved-player?discordUserId=test-user-1',
    'GET /players/discord?player=716515038',
    'GET /players/discord/saved-player?discordUserId=test-user-1',
    'GET /players/discord?player=716515038',
    'GET /players/discord/saved-player?discordUserId=test-user-1',
    'GET /players/716515038/matches?limit=10',
    'GET /players/discord/saved-player?discordUserId=test-user-1',
    'GET /live/players/716515038',
    'GET /players/discord/saved-player?discordUserId=test-user-1',
    'GET /players/716515038/loadouts',
  ]);
});

test('profile without an argument or saved player returns the actionable missing-player error', async () => {
  const requests: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, { fetchImpl: backendFetch(requests) });
  const handler = new CommandHandler(api, {} as RenderService, 'https://paladinscat.com');
  const profile = interaction('profile', 'test-user-2');

  await handler.handle(profile);

  assert.equal(
    profile.replies[0],
    'No player name was entered and you do not have a saved player. '
      + 'Enter a player or use `/save player:<name or ID>` first.',
  );
  assert.deepEqual(requests, [
    'GET /players/discord/saved-player?discordUserId=test-user-2',
  ]);
});

test('an explicit profile player overrides the saved-player lookup', async () => {
  const requests: string[] = [];
  const api = new PaladinsCatApi('http://backend:3005', 1000, { fetchImpl: backendFetch(requests) });
  const handler = new CommandHandler(api, {} as RenderService, 'https://paladinscat.com');
  const profile = interaction('profile', 'test-user-1', { player: 'OtherPlayer' });

  await handler.handle(profile);

  assert.deepEqual(requests, ['GET /players/discord?player=OtherPlayer']);
  const embed = (profile.replies[0] as { embeds: Array<{ title: string }> }).embeds[0];
  assert.ok(embed);
  assert.equal(embed.title, 'OtherPlayer');
});
