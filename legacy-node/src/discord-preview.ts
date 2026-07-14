import type { APIEmbed } from 'discord.js';
import { validateDiscordMessage, type DiscordMessagePayload } from './discord-message.js';
import { DEFAULT_PLAYER_AVATAR_PATH } from './player-profile-message.js';

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;',
  })[character] ?? character);
}

// This deliberately supports the compact Markdown subset used by the bot
// cards. The raw payload is shown alongside it, so Discord remains the final
// rendering authority while this stays a safe, faithful design preview.
function markdown(value: string | undefined): string {
  const escaped = escapeHtml(value ?? '')
    .replace(/\\([\\`*_~])/g, '$1')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(?<!\*)\*([^*]+)\*(?!\*)/g, '<em>$1</em>');
  return escaped.replace(/\n/g, '<br>');
}

function embedTextLength(embed: APIEmbed): number {
  return [
    embed.title, embed.description, embed.author?.name, embed.footer?.text,
    ...(embed.fields ?? []).flatMap((field) => [field.name, field.value]),
  ].filter(Boolean).join('').length;
}

function renderThumbnail(url: string | undefined): string {
  if (!url) return '';
  const image = `<img class="thumbnail" src="${escapeHtml(url)}" alt="Profile thumbnail">`;
  const pathname = url.split(/[?#]/, 1)[0] ?? '';
  if (!pathname.endsWith(DEFAULT_PLAYER_AVATAR_PATH)) return image;

  const avifUrl = url.replace(/Avatar_Default_Icon\.png(?=([?#]|$))/i, 'Avatar_Default_Icon.avif');
  return `<picture><source srcset="${escapeHtml(avifUrl)}" type="image/avif">${image}</picture>`;
}

function renderEmbed(embed: APIEmbed): string {
  const fields = (embed.fields ?? []).map((field) => `
    <section class="field${field.inline ? ' inline' : ''}">
      <h3>${markdown(field.name)}</h3>
      <p>${markdown(field.value)}</p>
    </section>`).join('');
  return `<article class="embed" style="--accent:#${(embed.color ?? 0x2dd4a3).toString(16).padStart(6, '0')}">
    ${embed.author?.name ? `<div class="author">${markdown(embed.author.name)}</div>` : ''}
    <div class="embed-body">
      <div class="copy">
        ${embed.title ? `<h2>${markdown(embed.title)}</h2>` : ''}
        ${embed.description ? `<p class="description">${markdown(embed.description)}</p>` : ''}
        ${fields ? `<div class="fields">${fields}</div>` : ''}
      </div>
      ${renderThumbnail(embed.thumbnail?.url)}
    </div>
    ${embed.footer?.text ? `<footer>${markdown(embed.footer.text)}</footer>` : ''}
  </article>`;
}

export function renderDiscordPreview(payload: DiscordMessagePayload): string {
  const errors = validateDiscordMessage(payload);
  const embeds = payload.embeds ?? [];
  const counters = embeds.map((embed, index) => `Embed ${index + 1}: ${embedTextLength(embed)} / 6000`).join(' · ');
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>PaladinsCat Discord preview</title><style>
  :root { color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; background:#1e1f22; color:#dbdee1; }
  body { margin:0; padding:32px; background:#313338; } main { max-width:760px; margin:auto; }
  header { display:flex; justify-content:space-between; align-items:baseline; gap:16px; margin-bottom:16px; }
  h1 { font-size:18px; margin:0; } .limits { font:12px ui-monospace, monospace; color:#949ba4; text-align:right; }
  .message { display:flex; gap:12px; padding:12px; background:#313338; } .bot-avatar { width:40px; height:40px; border-radius:50%; background:#2dd4a3; color:#111; display:grid; place-items:center; font-weight:800; }
  .content { min-width:0; flex:1; } .sender { font-weight:700; } .tag { color:#fff; background:#5865f2; border-radius:3px; font-size:10px; padding:1px 3px; margin-left:5px; }
  .embed { margin-top:6px; max-width:620px; border-left:4px solid var(--accent); border-radius:4px; background:#2b2d31; padding:10px 12px; }
  .author, footer { color:#b5bac1; font-size:12px; } .embed-body { display:flex; gap:18px; justify-content:space-between; } .copy { min-width:0; flex:1; }
  h2 { font-size:16px; margin:6px 0; color:#f2f3f5; } .description, .field p { margin:5px 0; line-height:1.35; } .fields { display:flex; flex-wrap:wrap; gap:12px 18px; margin-top:12px; }
  .field { width:100%; } .field.inline { width:calc(33.333% - 12px); min-width:145px; } .field h3 { font-size:12px; margin:0; color:#f2f3f5; } .field p { font-size:13px; white-space:normal; }
  .thumbnail { width:80px; height:80px; border-radius:4px; object-fit:cover; } code { background:#1e1f22; padding:1px 3px; border-radius:3px; } details { margin-top:24px; color:#b5bac1; } pre { overflow:auto; padding:12px; background:#1e1f22; border-radius:6px; font-size:12px; }
  .error { color:#ffb4ab; } @media (max-width:560px) { body { padding:12px; } .field.inline { width:100%; } }
</style></head><body><main>
<header><h1>Discord message preview</h1><div class="limits">${escapeHtml(counters || 'No embeds')}<br>Mentions disabled</div></header>
<div class="message"><div class="bot-avatar">PC</div><div class="content"><div><span class="sender">PaladinsCat</span><span class="tag">APP</span></div>
${payload.content ? `<p>${markdown(payload.content)}</p>` : ''}${embeds.map(renderEmbed).join('')}</div></div>
${errors.length > 0 ? `<p class="error">Validation: ${escapeHtml(errors.join(' · '))}</p>` : ''}
<details><summary>Exact Discord payload</summary><pre>${escapeHtml(JSON.stringify(payload, null, 2))}</pre></details>
</main></body></html>`;
}
