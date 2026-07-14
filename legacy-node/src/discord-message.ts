import type { APIEmbed } from 'discord.js';

export const DISCORD_MESSAGE_LIMITS = {
  content: 2000,
  embeds: 10,
  embedTitle: 256,
  embedDescription: 4096,
  embedFields: 25,
  embedFieldName: 256,
  embedFieldValue: 1024,
  embedFooter: 2048,
  embedAuthor: 256,
  embedsCombined: 6000,
} as const;

export type DiscordMessagePayload = {
  content?: string;
  embeds?: APIEmbed[];
  // Preview and bot replies are intentionally mention-safe. Any rich text
  // from a player profile must never notify Discord users or roles.
  allowedMentions: { parse: [] };
};

function textLength(value: string | undefined): number {
  return value?.trim().length ?? 0;
}

export function validateDiscordMessage(payload: DiscordMessagePayload): string[] {
  const errors: string[] = [];
  if (textLength(payload.content) > DISCORD_MESSAGE_LIMITS.content) {
    errors.push(`content exceeds ${DISCORD_MESSAGE_LIMITS.content} characters`);
  }
  if ((payload.embeds?.length ?? 0) > DISCORD_MESSAGE_LIMITS.embeds) {
    errors.push(`message contains more than ${DISCORD_MESSAGE_LIMITS.embeds} embeds`);
  }

  let combinedLength = 0;
  for (const [index, embed] of (payload.embeds ?? []).entries()) {
    const prefix = `embed ${index + 1}`;
    const titleLength = textLength(embed.title);
    const descriptionLength = textLength(embed.description);
    const footerLength = textLength(embed.footer?.text);
    const authorLength = textLength(embed.author?.name);
    combinedLength += titleLength + descriptionLength + footerLength + authorLength;

    if (titleLength > DISCORD_MESSAGE_LIMITS.embedTitle) errors.push(`${prefix} title exceeds ${DISCORD_MESSAGE_LIMITS.embedTitle} characters`);
    if (descriptionLength > DISCORD_MESSAGE_LIMITS.embedDescription) errors.push(`${prefix} description exceeds ${DISCORD_MESSAGE_LIMITS.embedDescription} characters`);
    if (footerLength > DISCORD_MESSAGE_LIMITS.embedFooter) errors.push(`${prefix} footer exceeds ${DISCORD_MESSAGE_LIMITS.embedFooter} characters`);
    if (authorLength > DISCORD_MESSAGE_LIMITS.embedAuthor) errors.push(`${prefix} author exceeds ${DISCORD_MESSAGE_LIMITS.embedAuthor} characters`);
    if ((embed.fields?.length ?? 0) > DISCORD_MESSAGE_LIMITS.embedFields) errors.push(`${prefix} contains more than ${DISCORD_MESSAGE_LIMITS.embedFields} fields`);

    for (const [fieldIndex, field] of (embed.fields ?? []).entries()) {
      const fieldPrefix = `${prefix} field ${fieldIndex + 1}`;
      const nameLength = textLength(field.name);
      const valueLength = textLength(field.value);
      combinedLength += nameLength + valueLength;
      if (nameLength > DISCORD_MESSAGE_LIMITS.embedFieldName) errors.push(`${fieldPrefix} name exceeds ${DISCORD_MESSAGE_LIMITS.embedFieldName} characters`);
      if (valueLength > DISCORD_MESSAGE_LIMITS.embedFieldValue) errors.push(`${fieldPrefix} value exceeds ${DISCORD_MESSAGE_LIMITS.embedFieldValue} characters`);
    }
  }

  if (combinedLength > DISCORD_MESSAGE_LIMITS.embedsCombined) {
    errors.push(`combined embed text exceeds ${DISCORD_MESSAGE_LIMITS.embedsCombined} characters`);
  }
  if (!payload.content && (payload.embeds?.length ?? 0) === 0) errors.push('message has no content or embeds');
  return errors;
}

export function assertDiscordMessage(payload: DiscordMessagePayload): DiscordMessagePayload {
  const errors = validateDiscordMessage(payload);
  if (errors.length > 0) throw new Error(`Discord message validation failed: ${errors.join('; ')}`);
  return payload;
}
