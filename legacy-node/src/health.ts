import http from 'node:http';
import { PaladinsCatApi } from './api-client.js';
import { RenderService } from './render-service.js';

export function startHealthServer(port: number, renders: RenderService, api: PaladinsCatApi, state: () => Record<string, unknown>) {
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://renderer.local');
    if (request.method === 'GET' && url.pathname === '/health') {
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end(JSON.stringify({ status: 'healthy', ...state(), render: renders.snapshot(), timestamp: new Date().toISOString() }));
      return;
    }
    const imageMatch = url.pathname.match(/^\/matches\/(\d{6,20})\/image$/);
    if (request.method === 'GET' && imageMatch?.[1]) {
      try {
        const image = await renders.match(await api.match(imageMatch[1]));
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
