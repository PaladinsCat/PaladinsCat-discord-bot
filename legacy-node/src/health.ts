import http from 'node:http';
import { RenderService } from './render-service.js';

export function startHealthServer(port: number, renders: RenderService, state: () => Record<string, unknown>) {
  const server = http.createServer((request, response) => {
    if (request.url !== '/health') { response.writeHead(404).end(); return; }
    response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    response.end(JSON.stringify({ status: 'healthy', ...state(), render: renders.snapshot(), timestamp: new Date().toISOString() }));
  });
  server.listen(port, '0.0.0.0');
  return server;
}
