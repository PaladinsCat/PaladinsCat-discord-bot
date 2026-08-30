import { Client, Events, GatewayIntentBits, REST } from 'discord.js';
import { loadConfig } from './config.js';
import { PaladinsCatApi } from './api-client.js';
import { AssetCatalog } from './asset-catalog.js';
import { MatchRenderer } from './match-renderer.js';
import { RenderService } from './render-service.js';
import { commandData, CommandHandler } from './commands.js';
import { startHealthServer } from './health.js';
import { syncDiscordCommands } from './command-registration.js';
import { ServiceTokenProvider } from './service-auth.js';

const config = loadConfig();
const api = new PaladinsCatApi(config.apiUrl, 12000, {
  localOnly: config.localOnly,
  matchTimeoutMs: config.matchLookupTimeoutMs,
  serviceAuth: config.serviceAuth ? new ServiceTokenProvider(config.serviceAuth) : undefined,
});
const renderer = new MatchRenderer(new AssetCatalog(config.assetRoot));
const renders = new RenderService(renderer, {
  concurrency: config.renderConcurrency,
  queueLimit: config.renderQueueLimit,
  timeoutMs: config.renderTimeoutMs,
  lookupConcurrency: config.matchLookupConcurrency,
  lookupQueueLimit: config.matchLookupQueueLimit,
  lookupTimeoutMs: config.matchLookupTimeoutMs,
  cacheBytes: config.renderCacheBytes,
  cacheTtlMs: config.renderCacheTtlMs,
});
let discordState = config.mode === 'dummy' ? 'dummy' : 'starting';
const health = startHealthServer(config.healthPort, renders, api, config.webUrl, () => ({ mode: config.mode, discord: discordState }));
let client: Client | null = null;
void renders.warm()
  .then(() => console.log('[bot] Chromium renderer warmed'))
  .catch((error) => console.warn(`[bot] Chromium warm-up failed: ${error instanceof Error ? error.message : error}`));

if (config.mode === 'dummy') {
  console.log(`[bot] dummy mode; health listening on ${config.healthPort}`);
} else {
  client = new Client({ intents: [GatewayIntentBits.Guilds] });
  const commands = new CommandHandler(api, renders, config.webUrl);

  client.once(Events.ClientReady, async (ready) => {
    discordState = 'ready';
    console.log(`[bot] connected as ${ready.user.tag}`);
    const rest = new REST({ version: '10' }).setToken(config.discordToken!);
    const registration = await syncDiscordCommands(
      rest,
      config.applicationId!,
      config.developmentGuildId,
      ready.guilds.cache.keys(),
      commandData,
    );
    console.log(`[bot] registered ${registration.registered} slash commands ${registration.scope === 'guild' ? 'in development guild' : 'globally'}`);
    if (registration.scope === 'global') {
      console.log(`[bot] cleared ${registration.clearedGuildScopes} stale guild command scope(s)`);
      if (registration.failedGuildScopes > 0) {
        console.warn(`[bot] failed to clear ${registration.failedGuildScopes} stale guild command scope(s)`);
      }
    }
    void commands.warmAutocomplete()
      .then(() => console.log('[bot] champion autocomplete catalog warmed'))
      .catch((error) => console.warn(`[bot] champion autocomplete warm-up failed: ${error instanceof Error ? error.message : error}`));
  });

  client.on(Events.InteractionCreate, async (interaction) => {
    if (interaction.isAutocomplete()) {
      await commands.handleAutocomplete(interaction);
      return;
    }
    if (interaction.isStringSelectMenu()) {
      await commands.handleComponent(interaction);
      return;
    }
    if (interaction.isChatInputCommand()) await commands.handle(interaction);
  });
  client.on(Events.Error, (error) => console.error('[bot] Discord client error', error));
  await client.login(config.discordToken);

}

let stopping = false;
const stop = () => {
  if (stopping) return;
  stopping = true;
  discordState = 'stopping';
  client?.destroy();
  health.closeAllConnections();
  const healthClosed = new Promise<void>((resolve) => health.close(() => resolve()));
  void Promise.allSettled([healthClosed, renders.close()]).then(() => process.exit(0));
  setTimeout(() => process.exit(0), 3000).unref();
};
process.once('SIGTERM', stop);
process.once('SIGINT', stop);
