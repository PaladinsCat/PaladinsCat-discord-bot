import path from 'node:path';
import { z } from 'zod';

const schema = z.object({
  PALADINSCAT_BOT_MODE: z.enum(['dummy', 'real']).default('dummy'),
  PALADINSCAT_API_URL: z.string().url().default('http://localhost:3304'),
  PALADINSCAT_WEB_URL: z.string().url().default('http://localhost:3000'),
  PALADINSCAT_BOT_LOCAL_ONLY: z.enum(['true', 'false']).default('true').transform((value) => value === 'true'),
  PALADINSCAT_ASSET_ROOT: z.string().default('../frontend/public/images'),
  PALADINSCAT_BOT_HEALTH_PORT: z.coerce.number().int().min(1).max(65535).default(3020),
  PALADINSCAT_RENDER_CONCURRENCY: z.coerce.number().int().min(1).max(2).default(1),
  PALADINSCAT_RENDER_QUEUE_LIMIT: z.coerce.number().int().min(1).max(50).default(10),
  PALADINSCAT_RENDER_TIMEOUT_MS: z.coerce.number().int().min(1000).max(120000).default(20000),
  PALADINSCAT_RENDER_CACHE_BYTES: z.coerce.number().int().min(0).max(64 * 1024 * 1024).default(32 * 1024 * 1024),
  PALADINSCAT_RENDER_CACHE_TTL_MS: z.coerce.number().int().min(1000).max(3600000).default(600000),
  DISCORD_TOKEN: z.string().optional(),
  DISCORD_APPLICATION_ID: z.string().optional(),
  DISCORD_DEVELOPMENT_GUILD_ID: z.string().optional(),
});

export type BotConfig = ReturnType<typeof loadConfig>;

export function loadConfig(env: NodeJS.ProcessEnv = process.env) {
  const parsed = schema.parse(env);
  if (parsed.PALADINSCAT_BOT_MODE === 'real' && (!parsed.DISCORD_TOKEN || !parsed.DISCORD_APPLICATION_ID)) {
    throw new Error('DISCORD_TOKEN and DISCORD_APPLICATION_ID are required in real mode');
  }
  return {
    mode: parsed.PALADINSCAT_BOT_MODE,
    apiUrl: parsed.PALADINSCAT_API_URL.replace(/\/$/, ''),
    webUrl: parsed.PALADINSCAT_WEB_URL.replace(/\/$/, ''),
    localOnly: parsed.PALADINSCAT_BOT_LOCAL_ONLY,
    assetRoot: path.resolve(process.cwd(), parsed.PALADINSCAT_ASSET_ROOT),
    healthPort: parsed.PALADINSCAT_BOT_HEALTH_PORT,
    renderConcurrency: parsed.PALADINSCAT_RENDER_CONCURRENCY,
    renderQueueLimit: parsed.PALADINSCAT_RENDER_QUEUE_LIMIT,
    renderTimeoutMs: parsed.PALADINSCAT_RENDER_TIMEOUT_MS,
    renderCacheBytes: parsed.PALADINSCAT_RENDER_CACHE_BYTES,
    renderCacheTtlMs: parsed.PALADINSCAT_RENDER_CACHE_TTL_MS,
    discordToken: parsed.DISCORD_TOKEN,
    applicationId: parsed.DISCORD_APPLICATION_ID,
    developmentGuildId: parsed.DISCORD_DEVELOPMENT_GUILD_ID,
  };
}
