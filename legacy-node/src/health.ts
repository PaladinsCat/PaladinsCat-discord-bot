import http from 'node:http';
import { PaladinsCatApi } from './api-client.js';
import { renderDiscordPreview } from './discord-preview.js';
import { buildPlayerProfileMessage } from './player-profile-message.js';
import { RenderService } from './render-service.js';

export function startHealthServer(port: number, renders: RenderService, api: PaladinsCatApi, webUrl: string, state: () => Record<string, unknown>) {
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://renderer.local');
    if (request.method === 'GET' && url.pathname === '/health') {
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end(JSON.stringify({ status: 'healthy', ...state(), render: renders.snapshot(), timestamp: new Date().toISOString() }));
      return;
    }
    const previewMatch = url.pathname.match(/^\/preview\/player\/(\d{1,20})(?:\.json)?$/);
    if (request.method === 'GET' && previewMatch?.[1]) {
      try {
        const profile = await api.playerById(previewMatch[1]);
        const payload = buildPlayerProfileMessage(profile, webUrl);
        const wantsJson = previewMatch[0].endsWith('.json') || url.searchParams.get('format') === 'json';
        response.writeHead(200, { 'content-type': wantsJson ? 'application/json; charset=utf-8' : 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(wantsJson ? JSON.stringify(payload) : renderDiscordPreview(payload));
      } catch (error) {
        response.writeHead(502, { 'content-type': 'application/json', 'cache-control': 'no-store' });
        response.end(JSON.stringify({ error: error instanceof Error ? error.message : 'Could not build this preview.' }));
      }
      return;
    }
    const imageMatch = url.pathname.match(/^\/matches\/(\d{6,20})\/image$/);
    if (request.method === 'GET' && imageMatch?.[1]) {
      try {
        const image = await renders.matchById(imageMatch[1], () => api.match(imageMatch[1]!));
        response.writeHead(200, { 'content-type': 'image/png', 'cache-control': 'private, max-age=60' });
        response.end(image);
      } catch (error) {
        response.writeHead(502, { 'content-type': 'application/json', 'cache-control': 'no-store' });
        response.end(JSON.stringify({ error: error instanceof Error ? error.message : 'Could not render this match.' }));
      }
      return;
    }
    response.writeHead(404).end();
  });
  server.listen(port, '0.0.0.0');
  return server;
}
