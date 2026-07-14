import {
  AttachmentBuilder,
  AutocompleteInteraction,
  ChatInputCommandInteraction,
  EmbedBuilder,
  SlashCommandStringOption,
  SlashCommandBuilder,
} from 'discord.js';
import { PaladinsCatApi, PaladinsCatApiError } from './api-client.js';
import { buildPlayerProfileMessage } from './player-profile-message.js';
import { RenderService } from './render-service.js';
import { QueueFullError } from './render-queue.js';
import type { Champion } from './types.js';

const accent = 0x2dd4a3;
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
  new SlashCommandBuilder().setName('loadouts').setDescription('List a player’s saved loadouts')
    .addStringOption((option) => option.setName('player').setDescription('Player name or ID').setRequired(true)),
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
      if (interaction.commandName === 'help') return interaction.reply({ embeds: [this.helpEmbed()], ephemeral: true });
      await interaction.deferReply();
      switch (interaction.commandName) {
        case 'player': return this.player(interaction);
        case 'match': return this.match(interaction);
        case 'history': return this.history(interaction);
        case 'current': return this.current(interaction);
        case 'loadouts': return this.loadouts(interaction);
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

  private async player(interaction: ChatInputCommandInteraction) {
    const resolved = await this.api.resolvePlayer(interaction.options.getString('player', true));
    const [response, recentMatches] = await Promise.all([
      this.api.playerById(resolved.id),
      this.api.playerHistoryById(resolved.id, 5).catch(() => []),
    ]);
    return interaction.editReply(buildPlayerProfileMessage(response, recentMatches, this.webUrl));
  }

  private async match(interaction: ChatInputCommandInteraction) {
    const now = Date.now();
    const previous = this.imageCooldowns.get(interaction.user.id) ?? 0;
    if (previous > now) throw new Error(`Image cooldown: try again in ${Math.ceil((previous - now) / 1000)}s.`);
    this.imageCooldowns.set(interaction.user.id, now + 10000);
    const id = interaction.options.getString('id', true);
    if (!/^\d{6,20}$/.test(id)) throw new Error('Enter a valid numeric match ID.');
    const record = await this.api.match(id);
    const buffer = await this.renders.match(record);
    const attachment = new AttachmentBuilder(buffer, { name: `paladinscat-match-${id}.png`, description: `Paladins match ${id}` });
    return interaction.editReply({ content: `${this.webUrl}/matches/${id}`, files: [attachment] });
  }

  private async history(interaction: ChatInputCommandInteraction) {
    const input = interaction.options.getString('player', true);
    const player = await this.api.resolvePlayer(input);
    const rows = await this.api.playerHistory(input, 10);
    const lines = rows.slice(0, 10).map((row: any) => `${row.win_status === 'Winner' ? '✅' : '❌'} **${row.champion_name ?? 'Unknown'}** · ${row.kills ?? 0}/${row.deaths ?? 0}/${row.assists ?? 0} · [${row.match_id}](${this.webUrl}/matches/${row.match_id})`);
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle(`${player.name} · Recent matches`).setURL(`${this.webUrl}/players/${player.id}`).setDescription(lines.join('\n') || 'No recent matches found.')] });
  }

  private async current(interaction: ChatInputCommandInteraction) {
    const input = interaction.options.getString('player', true);
    const result = await this.api.liveMatch(input);
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle('Current match').setDescription(`\`\`\`json\n${JSON.stringify(result, null, 2).slice(0, 3500)}\n\`\`\``)] });
  }

  private async loadouts(interaction: ChatInputCommandInteraction) {
    const input = interaction.options.getString('player', true);
    const player = await this.api.resolvePlayer(input);
    const payload = await this.api.playerLoadouts(input) as any;
    const rows = Array.isArray(payload.loadouts) ? payload.loadouts : [];
    const lines = rows.slice(0, 15).map((row: any) => `• **${row.champion_name ?? 'Champion'}** · ${row.loadout_name ?? 'Unnamed'}`);
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle(`${player.name} · Loadouts`).setURL(`${this.webUrl}/players/${player.id}/loadouts`).setDescription(lines.join('\n') || 'No saved loadouts found.')] });
  }

  private async champion(interaction: ChatInputCommandInteraction) {
    const name = interaction.options.getString('champion', true);
    const result: any = await this.api.champion(name.toLocaleLowerCase());
    const champion = result.champion ?? {};
    const stats = result.stats ?? {};
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle(champion.name ?? name).setURL(`${this.webUrl}/champions/${encodeURIComponent(String(champion.name ?? name).toLocaleLowerCase())}`).setDescription(champion.title ?? '').addFields(
      { name: 'Class', value: champion.roles ?? 'Unknown', inline: true },
      { name: 'Win rate', value: stats.win_rate == null ? '—' : `${Number(stats.win_rate).toFixed(1)}%`, inline: true },
      { name: 'Ranked matches', value: Number(stats.total_matches ?? 0).toLocaleString(), inline: true },
    )] });
  }

  private async leaderboard(interaction: ChatInputCommandInteraction) {
    const rows = await this.api.rankedLeaderboard(10);
    const lines = rows.map((row: any, index) => `**${index + 1}.** [${row.name}](${this.webUrl}/players/${row.player_id}) · ${Number(row.points ?? 0).toLocaleString()} TP`);
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle('Ranked leaderboard').setURL(`${this.webUrl}/players/leaderboard`).setDescription(lines.join('\n') || 'No ranked players found.')] });
  }

  private async random(interaction: ChatInputCommandInteraction) {
    const role = interaction.options.getString('role');
    const champions = (await this.api.champions()).filter((champion) => !role || String(champion.roles ?? '').toLocaleLowerCase().replace(/\s/g, '').includes(role));
    const selected = champions[Math.floor(Math.random() * champions.length)];
    if (!selected) throw new Error('No champion matched that class.');
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle(selected.name).setURL(`${this.webUrl}/champions/${encodeURIComponent(selected.name.toLocaleLowerCase())}`).setDescription(`${selected.roles ?? 'Champion'} · ${selected.title ?? ''}`)] });
  }

  private async status(interaction: ChatInputCommandInteraction) {
    const start = performance.now();
    const api = await this.api.status();
    const latency = Math.round(performance.now() - start);
    const state = this.renders.snapshot();
    return interaction.editReply({ embeds: [new EmbedBuilder().setColor(accent).setTitle('PaladinsCat status').addFields(
      { name: 'API', value: `${(api as any).status ?? 'online'} · ${latency}ms`, inline: true },
      { name: 'Render queue', value: `${state.queue.active} active · ${state.queue.queued} queued`, inline: true },
      { name: 'Render cache', value: `${state.cache.entries} images · ${(state.cache.bytes / 1048576).toFixed(1)} MiB`, inline: true },
    )] });
  }

  private helpEmbed() {
    return new EmbedBuilder().setColor(accent).setTitle('PaladinsCat commands').setDescription([
      '`/player` profile, rank, record and performance', '`/match` optimized match-result image',
      '`/history` recent matches', '`/current` current live match', '`/loadouts` saved decks',
      '`/champion` champion ranked statistics', '`/leaderboard` top ranked players',
      '`/random` random champion by optional class', '`/status` API and renderer health',
    ].join('\n'));
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

  private errorMessage(error: unknown) {
    if (error instanceof QueueFullError) return 'The image queue is full. Try again shortly.';
    if (error instanceof PaladinsCatApiError) return error.message;
    return error instanceof Error ? error.message : 'The command could not be completed.';
  }
}
