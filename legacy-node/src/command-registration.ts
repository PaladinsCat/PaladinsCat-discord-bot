import { Routes, type REST } from 'discord.js';

type RegistrationResult = {
  scope: 'global' | 'guild';
  registered: number;
  clearedGuildScopes: number;
  failedGuildScopes: number;
};

export async function syncDiscordCommands(
  rest: Pick<REST, 'put'>,
  applicationId: string,
  developmentGuildId: string | undefined,
  connectedGuildIds: Iterable<string>,
  commands: unknown[],
): Promise<RegistrationResult> {
  if (developmentGuildId) {
    await rest.put(Routes.applicationGuildCommands(applicationId, developmentGuildId), { body: commands });
    return { scope: 'guild', registered: commands.length, clearedGuildScopes: 0, failedGuildScopes: 0 };
  }

  await rest.put(Routes.applicationCommands(applicationId), { body: commands });

  // Global production commands supersede any development-guild copies. Empty
  // every connected guild scope so Discord cannot surface both registrations
  // under the same application while its command caches converge.
  const guildIds = [...new Set(connectedGuildIds)];
  const cleared = await Promise.allSettled(guildIds.map((guildId) => (
    rest.put(Routes.applicationGuildCommands(applicationId, guildId), { body: [] })
  )));
  const clearedGuildScopes = cleared.filter((result) => result.status === 'fulfilled').length;
  return {
    scope: 'global',
    registered: commands.length,
    clearedGuildScopes,
    failedGuildScopes: cleared.length - clearedGuildScopes,
  };
}
