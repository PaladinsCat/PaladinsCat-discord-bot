import { Client, Events, GatewayIntentBits, REST, Routes } from 'discord.js';
import { loadConfig } from './config.js';
import { PaladinsCatApi } from './api-client.js';
import { AssetCatalog } from './asset-catalog.js';
import { MatchRenderer } from './match-renderer.js';
import { RenderService } from './render-service.js';
import { commandData, CommandHandler } from './commands.js';
import { startHealthServer } from './health.js';

const config = loadConfig();
const api = new PaladinsCatApi(config.apiUrl, 12000, { localOnly: config.localOnly });
const renders = new RenderService(new MatchRenderer(new AssetCatalog(config.assetRoot)), {
  concurrency: config.renderConcurrency,
  queueLimit: config.renderQueueLimit,
  timeoutMs: config.renderTimeoutMs,
  cacheBytes: config.renderCacheBytes,
  cacheTtlMs: config.renderCacheTtlMs,
});
let discordState = config.mode === 'dummy' ? 'dummy' : 'starting';
const health = startHealthServer(config.healthPort, renders, () => ({ mode: config.mode, discord: discordState }));
let client: Client | null = null;

if (config.mode === 'dummy') {
  console.log(`[bot] dummy mode; health listening on ${config.healthPort}`);
} else {
  client = new Client({ intents: [GatewayIntentBits.Guilds] });
  const commands = new CommandHandler(api, renders, config.webUrl);

  client.once(Events.ClientReady, async (ready) => {
    discordState = 'ready';
    console.log(`[bot] connected as ${ready.user.tag}`);
    const rest = new REST({ version: '10' }).setToken(config.discordToken!);
    const route = config.developmentGuildId
      ? Routes.applicationGuildCommands(config.applicationId!, config.developmentGuildId)
      : Routes.applicationCommands(config.applicationId!);
    await rest.put(route, { body: commandData });
    console.log(`[bot] registered ${commandData.length} slash commands${config.developmentGuildId ? ' in development guild' : ' globally'}`);
  });

  client.on(Events.InteractionCreate, async (interaction) => {
    if (interaction.isChatInputCommand()) await commands.handle(interaction);
  });
  client.on(Events.Error, (error) => console.error('[bot] Discord client error', error));
  await client.login(config.discordToken);

}

const stop = () => {
  discordState = 'stopping';
  client?.destroy();
  health.closeAllConnections();
  health.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1500);
};
process.once('SIGTERM', stop);
process.once('SIGINT', stop);
