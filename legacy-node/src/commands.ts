import { randomUUID } from 'node:crypto';
import {
  ActionRowBuilder,
  AttachmentBuilder,
  AutocompleteInteraction,
  ChatInputCommandInteraction,
  SlashCommandStringOption,
  SlashCommandBuilder,
  StringSelectMenuBuilder,
  StringSelectMenuInteraction,
} from 'discord.js';
import { PaladinsCatApi, PaladinsCatApiError } from './api-client.js';
import { buildPlayerProfileMessage } from './player-profile-message.js';
import {
  buildChampionPayload,
  buildCurrentPayload,
  buildHelpPayload,
  buildHistoryPayload,
  buildLeaderboardPayload,
  buildLoadoutSelectionPayload,
  buildNoLoadoutsPayload,
  buildRandomPayload,
  buildStatusPayload,
} from './message-builders.js';
import { findPlayerChampionLoadouts } from './loadout-service.js';
import { RenderService } from './render-service.js';
import { QueueFullError } from './render-queue.js';
import type { Champion, PlayerLoadout, PlayerSearchResult } from './types.js';

const LOADOUT_SESSION_TTL_MS = 5 * 60 * 1000;
const IMAGE_COOLDOWN_MS = 10 * 1000;

type LoadoutSession = {
  userId: string;
  player: PlayerSearchResult;
  loadouts: PlayerLoadout[];
  expiresAt: number;
};
const championOption = (option: SlashCommandStringOption) => option
  .setName('champion')
  .setDescription('Champion name')
  .setRequired(true)
  .setAutocomplete(true);

function normalized(value: string) {
  return value.normalize('NFKD').toLocaleLowerCase().replace(/[^a-z0-9]+/g, '');
}

export function championAutocompleteChoices(champions: Champion[], input: string) {
  const query = normalized(input);
  const unique = [...new Map(
    champions
      .filter((champion) => champion?.name)
      .map((champion) => [normalized(champion.name), champion] as const),
  ).values()];

  return unique
    .map((champion) => {
      const key = normalized(champion.name);
      const words = champion.name.split(/[^a-z0-9]+/i).map(normalized).filter(Boolean);
      const score = !query ? 4
        : key === query ? 0
          : key.startsWith(query) ? 1
            : words.some((word) => word.startsWith(query)) ? 2
              : key.includes(query) ? 3
                : Number.POSITIVE_INFINITY;
      return { champion, score };
    })
    .filter(({ score }) => Number.isFinite(score))
    .sort((left, right) => left.score - right.score || left.champion.name.localeCompare(right.champion.name))
    .slice(0, 25)
    .map(({ champion }) => ({ name: champion.name.slice(0, 100), value: champion.name.slice(0, 100) }));
}

export const commandData = [
  new SlashCommandBuilder().setName('help').setDescription('List PaladinsCat bot commands'),
  new SlashCommandBuilder().setName('player').setDescription('Show a Paladins player profile')
    .addStringOption((option) => option.setName('player').setDescription('Player name or ID').setRequired(true)),
  new SlashCommandBuilder().setName('match').setDescription('Render a match result image')
    .addStringOption((option) => option.setName('id').setDescription('Match ID').setRequired(true)),
  new SlashCommandBuilder().setName('history').setDescription('Show recent matches for a player')
    .addStringOption((option) => option.setName('player').setDescription('Player name or ID').setRequired(true)),
  new SlashCommandBuilder().setName('current').setDescription('Check a player’s current live match')
    .addStringOption((option) => option.setName('player').setDescription('Player name or ID').setRequired(true)),
  new SlashCommandBuilder().setName('loadout').setDescription('Render one of a player’s saved champion loadouts')
    .addStringOption((option) => option.setName('player').setDescription('Player name or ID').setRequired(true))
    .addStringOption(championOption),
  new SlashCommandBuilder().setName('champion').setDescription('Show champion ranked statistics')
    .addStringOption(championOption),
  new SlashCommandBuilder().setName('leaderboard').setDescription('Show the ranked leaderboard'),
  new SlashCommandBuilder().setName('random').setDescription('Choose a random champion')
    .addStringOption((option) => option.setName('role').setDescription('Optional class').addChoices(
      { name: 'Damage', value: 'damage' }, { name: 'Flank', value: 'flank' },
      { name: 'Frontline', value: 'frontline' }, { name: 'Support', value: 'support' },
    )),
  new SlashCommandBuilder().setName('status').setDescription('Show PaladinsCat API and render queue status'),
].map((command) => command.toJSON());

export class CommandHandler {
  private readonly imageCooldowns = new Map<string, number>();
  private readonly loadoutSessions = new Map<string, LoadoutSession>();
  private championCache: { values: Champion[]; expiresAt: number } | null = null;

  constructor(private readonly api: PaladinsCatApi, private readonly renders: RenderService, private readonly webUrl: string) {}

  async warmAutocomplete(): Promise<void> {
    await this.championsForAutocomplete();
  }

  async handleAutocomplete(interaction: AutocompleteInteraction): Promise<void> {
    const focused = interaction.options.getFocused(true);
    if (focused.name !== 'champion') {
      await interaction.respond([]);
      return;
    }

    try {
      const champions = await this.championsForAutocomplete();
      await interaction.respond(championAutocompleteChoices(champions, String(focused.value ?? '')));
    } catch (error) {
      console.warn(`[bot] champion autocomplete failed: ${error instanceof Error ? error.message : error}`);
      if (!interaction.responded) await interaction.respond([]);
    }
  }

  async handle(interaction: ChatInputCommandInteraction) {
    try {
      if (interaction.commandName === 'help') return interaction.reply({ ...buildHelpPayload(), ephemeral: true });
      await interaction.deferReply();
      switch (interaction.commandName) {
        case 'player': return this.player(interaction);
        case 'match': return this.match(interaction);
        case 'history': return this.history(interaction);
        case 'current': return this.current(interaction);
        case 'loadout': return this.loadout(interaction);
        case 'champion': return this.champion(interaction);
        case 'leaderboard': return this.leaderboard(interaction);
        case 'random': return this.random(interaction);
        case 'status': return this.status(interaction);
        default: return interaction.editReply('Unknown command. Use `/help`.');
      }
    } catch (error) {
      const message = this.errorMessage(error);
      if (interaction.deferred || interaction.replied) await interaction.editReply(message);
      else await interaction.reply({ content: message, ephemeral: true });
    }
  }

  async handleComponent(interaction: StringSelectMenuInteraction): Promise<void> {
    if (!interaction.customId.startsWith('loadout:')) return;
    try {
      this.pruneLoadoutSessions();
      const token = interaction.customId.slice('loadout:'.length);
      const session = this.loadoutSessions.get(token);
      if (!session || session.expiresAt <= Date.now()) {
        this.loadoutSessions.delete(token);
        await interaction.reply({ content: 'This loadout menu expired. Run `/loadout` again.', ephemeral: true });
        return;
      }
      if (session.userId !== interaction.user.id) {
        await interaction.reply({ content: 'Only the player who opened this menu can choose its loadout.', ephemeral: true });
        return;
      }
      const selectedId = interaction.values[0];
      const loadout = session.loadouts.find((row) => String(row.id) === selectedId);
      if (!loadout) {
        await interaction.reply({ content: 'That saved loadout is no longer available. Run `/loadout` again.', ephemeral: true });
        return;
      }

      this.claimImageCooldown(interaction.user.id);
      this.loadoutSessions.delete(token);
      await interaction.deferUpdate();
      const buffer = await this.renders.loadout({ player: session.player, loadout });
      const safeChampion = normalized(loadout.champion_name) || 'champion';
      const attachment = new AttachmentBuilder(buffer, {
        name: `paladinscat-loadout-${safeChampion}-${loadout.id}.png`,
        description: `${session.player.name}'s ${loadout.champion_name} loadout ${loadout.loadout_name}`,
      });
      await interaction.editReply({ content: '', embeds: [], components: [], files: [attachment] });
    } catch (error) {
      const message = this.errorMessage(error);
      if (interaction.deferred || interaction.replied) await interaction.editReply({ content: message, components: [] });
      else await interaction.reply({ content: message, ephemeral: true });
    }
  }

  private async player(interaction: ChatInputCommandInteraction) {
    const response = await this.api.discordPlayer(interaction.options.getString('player', true));
    return interaction.editReply(buildPlayerProfileMessage(response, this.webUrl));
  }

  private async match(interaction: ChatInputCommandInteraction) {
    this.claimImageCooldown(interaction.user.id);
    const id = interaction.options.getString('id', true);
    if (!/^\d{6,20}$/.test(id)) throw new Error('Enter a valid numeric match ID.');
    const buffer = await this.renders.matchById(id, () => this.api.match(id));
    const attachment = new AttachmentBuilder(buffer, { name: `paladinscat-match-${id}.png`, description: `Paladins match ${id}` });
    return interaction.editReply({ content: `${this.webUrl}/matches/${id}`, files: [attachment] });
  }

  private async history(interaction: ChatInputCommandInteraction) {
    const input = interaction.options.getString('player', true);
    const player = await this.api.resolvePlayer(input);
    const rows = await this.api.playerHistoryById(player.id, 10);
    return interaction.editReply(buildHistoryPayload(player.name, rows, this.webUrl));
  }

  private async current(interaction: ChatInputCommandInteraction) {
    const input = interaction.options.getString('player', true);
    const result = await this.api.liveMatch(input);
    return interaction.editReply(buildCurrentPayload(result, this.webUrl));
  }

  private async loadout(interaction: ChatInputCommandInteraction) {
    const result = await findPlayerChampionLoadouts(
      this.api,
      interaction.options.getString('player', true),
      interaction.options.getString('champion', true),
    );
    if (result.loadouts.length === 0) {
      return interaction.editReply(buildNoLoadoutsPayload(result.player.name, result.championName, result.refreshError));
    }

    this.pruneLoadoutSessions();
    const token = randomUUID();
    this.loadoutSessions.set(token, {
      userId: interaction.user.id,
      player: result.player,
      loadouts: result.loadouts.slice(0, 25),
      expiresAt: Date.now() + LOADOUT_SESSION_TTL_MS,
    });
    const select = new StringSelectMenuBuilder()
      .setCustomId(`loadout:${token}`)
      .setPlaceholder(`Choose a ${result.championName} loadout`)
      .addOptions(result.loadouts.slice(0, 25).map((loadout) => ({
        label: (loadout.loadout_name || 'Unnamed Loadout').slice(0, 100),
        description: `${loadout.card_levels.reduce((sum, level) => sum + Number(level || 0), 0)} card points`.slice(0, 100),
        value: String(loadout.id),
      })));
    const row = new ActionRowBuilder<StringSelectMenuBuilder>().addComponents(select);
    return interaction.editReply({
      ...buildLoadoutSelectionPayload(
        result.player.name,
        result.championName,
        result.loadouts,
        this.webUrl,
        result.player.id,
        result.refreshed,
      ),
      components: [row],
    });
  }

  private async champion(interaction: ChatInputCommandInteraction) {
    const name = interaction.options.getString('champion', true);
    const result: any = await this.api.champion(name.toLocaleLowerCase());
    return interaction.editReply(buildChampionPayload(result, this.webUrl));
  }

  private async leaderboard(interaction: ChatInputCommandInteraction) {
    const rows = await this.api.rankedLeaderboard(10);
    return interaction.editReply(buildLeaderboardPayload(rows, this.webUrl));
  }

  private async random(interaction: ChatInputCommandInteraction) {
    const role = interaction.options.getString('role');
    const champions = (await this.championsForAutocomplete()).filter((champion) => !role || String(champion.roles ?? '').toLocaleLowerCase().replace(/\s/g, '').includes(role));
    const selected = champions[Math.floor(Math.random() * champions.length)];
    if (!selected) throw new Error('No champion matched that class.');
    return interaction.editReply(buildRandomPayload(selected, this.webUrl, role ?? undefined));
  }

  private async status(interaction: ChatInputCommandInteraction) {
    const start = performance.now();
    const api = await this.api.status();
    const latency = Math.round(performance.now() - start);
    const state = this.renders.snapshot();
    const renderState = {
      active: state.queue.active,
      queued: state.queue.queued,
      durationMs: state.queue.durationMs,
      entries: state.cache.entries,
      bytes: state.cache.bytes,
      hits: state.cache.hits,
    };
    return interaction.editReply(buildStatusPayload(api, latency, renderState));
  }

  private async championsForAutocomplete(): Promise<Champion[]> {
    const now = Date.now();
    if (this.championCache && this.championCache.expiresAt > now) return this.championCache.values;
    try {
      const values = (await this.api.champions())
        .filter((champion) => champion?.name)
        .sort((left, right) => left.name.localeCompare(right.name));
      this.championCache = { values, expiresAt: now + 60 * 60 * 1000 };
      return values;
    } catch (error) {
      if (this.championCache) return this.championCache.values;
      throw error;
    }
  }

  private claimImageCooldown(userId: string): void {
    const now = Date.now();
    const previous = this.imageCooldowns.get(userId) ?? 0;
    if (previous > now) throw new Error(`Image cooldown: try again in ${Math.ceil((previous - now) / 1000)}s.`);
    this.imageCooldowns.set(userId, now + IMAGE_COOLDOWN_MS);
  }

  private pruneLoadoutSessions(): void {
    const now = Date.now();
    for (const [token, session] of this.loadoutSessions) {
      if (session.expiresAt <= now) this.loadoutSessions.delete(token);
    }
  }

  private errorMessage(error: unknown) {
    if (error instanceof QueueFullError) return 'The image queue is full. Try again shortly.';
    if (error instanceof PaladinsCatApiError) return error.message;
    return error instanceof Error ? error.message : 'The command could not be completed.';
  }
}
