import http from 'node:http';
import { PaladinsCatApi } from './api-client.js';
import { renderDiscordPreview } from './discord-preview.js';
import { buildPlayerProfileMessage } from './player-profile-message.js';
import {
  buildChampionPayload,
  buildCurrentPayload,
  buildHelpPayload,
  buildHistoryPayload,
  buildItemsPayload,
  buildLoadoutSelectionPayload,
  buildMapsPayload,
  buildNoLoadoutsPayload,
  buildCompositionPayload,
} from './message-builders.js';
import { RANKED_LOBBY_SCOPES, rankedLobbyScope } from './ranked-lobby.js';
import { findPlayerChampionLoadouts } from './loadout-service.js';
import { RenderService } from './render-service.js';

type PreviewParam = {
  name: string;
  label: string;
  required?: boolean;
  choices?: Array<{ label: string; value: string }>;
};

type PreviewCommand = { name: string; desc: string; params: PreviewParam[] };

const LOBBY_PREVIEW_PARAM: PreviewParam = {
  name: 'lobby',
  label: 'Lobby',
  required: true,
  choices: RANKED_LOBBY_SCOPES.map(({ label, value }) => ({ label, value })),
};

const COMMANDS: PreviewCommand[] = [
  { name: 'help', desc: 'List bot commands', params: [] },
  { name: 'player', desc: 'Player profile', params: [{ name: 'player', label: 'Player name or ID', required: true }] },
  { name: 'match', desc: 'Match result image', params: [{ name: 'id', label: 'Match ID', required: true }] },
  { name: 'history', desc: 'Recent matches', params: [{ name: 'player', label: 'Player name or ID', required: true }] },
  { name: 'current', desc: 'Current live match', params: [{ name: 'player', label: 'Player name or ID (`mock` for sample)', required: true }] },
  { name: 'loadout', desc: 'Choose and render a saved loadout', params: [{ name: 'player', label: 'Player name or ID', required: true }, { name: 'champion', label: 'Champion name', required: true }] },
  { name: 'champion', desc: 'Champion ranked stats by lobby tier', params: [{ name: 'champion', label: 'Champion name', required: true }, LOBBY_PREVIEW_PARAM] },
  { name: 'maps', desc: 'Statistics for every ranked map', params: [] },
  { name: 'composition', desc: 'Five most-played ranked team compositions', params: [] },
  { name: 'items', desc: 'Ranked item statistics by lobby tier', params: [LOBBY_PREVIEW_PARAM] },
];

const CURRENT_MATCH_MOCK = {
  player_id: '42',
  match: {
    match_id: '9001',
    queue_id: 486,
    map: 'Stone Keep',
    region: 'NA',
    source_player_id: '42',
    detected_at: '2026-07-21T22:00:00Z',
  },
  players: [
    { player_id: '42', player_name: 'Point_Tank', champion_name: 'Ash', kbm_tier: 26, profile_win_rate: 54.8, queue_elo: 1842.4, task_force: 1 },
    { player_id: '43', player_name: 'SolarFlare', champion_name: 'Furia', kbm_tier: 15, profile_win_rate: 51.2, queue_elo: 1518.7, task_force: 1 },
    { player_id: '45', player_name: 'Accelarate', champion_name: 'Androxus', kbm_tier: 24, profile_win_rate: 56.1, queue_elo: 1796.2, task_force: 1 },
    { player_id: '46', player_name: 'PrimalHunter', champion_name: 'Tyra', kbm_tier: 20, profile_win_rate: 49.7, queue_elo: 1621.5, task_force: 1 },
    { player_id: '47', player_name: 'HotWall', champion_name: 'Fernando', kbm_tier: 21, profile_win_rate: 52.9, queue_elo: 1704.1, task_force: 1 },
    { player_id: '44', player_name: 'NightStep', champion_name: 'Vatu', kbm_tier: 21, profile_win_rate: 55.4, queue_elo: 1758.9, task_force: 2 },
    { player_id: '-1', player_name: 'Private Account', champion_name: 'Io', task_force: 2 },
    { player_id: '48', player_name: 'ForgeFather', champion_name: 'Barik', kbm_tier: 26, profile_win_rate: 53.6, queue_elo: 1827.6, task_force: 2 },
    { player_id: '49', player_name: 'RoyalDetonator', champion_name: 'Bomb King', kbm_tier: 23, profile_win_rate: 50.5, queue_elo: 1733.2, task_force: 2 },
    { player_id: '50', player_name: 'MirageMaker', champion_name: 'Ying', kbm_tier: 19, profile_win_rate: 52.1, queue_elo: 1649.8, task_force: 2 },
  ],
};

function handlePreviewCommand(
  command: string,
  params: Record<string, string>,
  api: PaladinsCatApi,
  webUrl: string,
) {
  switch (command) {
    case 'help': return buildHelpPayload();
    case 'player': {
      const response = api.discordPlayer(params.player ?? '');
      return (async () => {
        const profile = await response;
        return buildPlayerProfileMessage(profile, webUrl);
      })();
    }
    case 'history': {
      const fetch = api.resolvePlayer(params.player ?? '');
      return (async () => {
        const player = await fetch;
        const rows = await api.playerHistoryById(player.id, 10);
        return buildHistoryPayload(player.name, rows, webUrl);
      })();
    }
    case 'current': {
      if ((params.player ?? '').trim().toLocaleLowerCase() === 'mock') {
        return buildCurrentPayload(CURRENT_MATCH_MOCK, webUrl);
      }
      const fetch = api.liveMatch(params.player ?? '');
      return (async () => {
        const result = await fetch;
        return buildCurrentPayload(result, webUrl);
      })();
    }
    case 'loadout': {
      const find = findPlayerChampionLoadouts(api, params.player ?? '', params.champion ?? '');
      return (async () => {
        const result = await find;
        if (result.loadouts.length === 0) {
          return buildNoLoadoutsPayload(result.player.name, result.championName, result.refreshError);
        }
        return {
          ...buildLoadoutSelectionPayload(result.player.name, result.championName, result.loadouts, webUrl, result.player.id, result.refreshed),
          components: [{
            type: 1 as const,
            components: [{
              type: 3 as const,
              custom_id: `preview-loadout:${result.player.id}`,
              placeholder: `Choose a ${result.championName} loadout`,
              options: result.loadouts.slice(0, 25).map((loadout) => ({
                label: (loadout.loadout_name || 'Unnamed Loadout').slice(0, 100),
                description: `${loadout.card_levels.reduce((sum, level) => sum + Number(level || 0), 0)} card points`,
                value: String(loadout.id),
              })),
            }],
          }],
        };
      })();
    }
    case 'champion': {
      const scope = rankedLobbyScope(params.lobby);
      const fetch = api.championPageData((params.champion ?? '').toLocaleLowerCase(), scope);
      return (async () => {
        const result = await fetch;
        return buildChampionPayload(result, webUrl, scope.label);
      })();
    }
    case 'maps': {
      const fetch = api.rankedMaps(100);
      return (async () => buildMapsPayload(await fetch, webUrl))();
    }
    case 'composition': {
      const fetch = api.rankedCompositions(5);
      return (async () => buildCompositionPayload(await fetch, webUrl))();
    }
    case 'items': {
      const scope = rankedLobbyScope(params.lobby);
      const fetch = api.rankedItems(scope, 20);
      return (async () => buildItemsPayload(await fetch, webUrl, scope.label))();
    }
    default: throw new Error(`Unknown command: ${command}`);
  }
}

export function startHealthServer(port: number, renders: RenderService, api: PaladinsCatApi, webUrl: string, state: () => Record<string, unknown>) {
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://preview.local');
    const pathname = url.pathname;

    if (request.method === 'GET' && pathname === '/health') {
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end(JSON.stringify({ status: 'healthy', ...state(), render: renders.snapshot(), timestamp: new Date().toISOString() }));
      return;
    }

    const previewMatch = pathname.match(/^\/preview\/player\/(\d{1,20})(?:\.json)?$/);
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

    const imageMatch = pathname.match(/^\/matches\/(\d{6,20})\/image$/);
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

    const loadoutImageMatch = pathname.match(/^\/preview\/loadout\/(\d{1,20})\/(\d{1,20})\/image$/);
    if (request.method === 'GET' && loadoutImageMatch?.[1] && loadoutImageMatch[2]) {
      try {
        const [profile, loadout] = await Promise.all([
          api.playerById(loadoutImageMatch[1]),
          api.playerLoadoutById(loadoutImageMatch[1], Number(loadoutImageMatch[2])),
        ]);
        const image = await renders.loadout({ player: profile.player, loadout });
        response.writeHead(200, { 'content-type': 'image/png', 'cache-control': 'private, max-age=60' });
        response.end(image);
      } catch (error) {
        response.writeHead(502, { 'content-type': 'application/json', 'cache-control': 'no-store' });
        response.end(JSON.stringify({ error: error instanceof Error ? error.message : 'Could not render this loadout.' }));
      }
      return;
    }

    if (pathname === '/preview' || pathname === '/') {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
      response.end(renderPlaygroundHTML());
      return;
    }

    if (pathname === '/preview/commands') {
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end(JSON.stringify({ commands: COMMANDS }));
      return;
    }

    const cmdMatch = pathname.match(/^\/preview\/cmd\/(.*?)(?:\.json)?$/);
    if (request.method === 'GET' && cmdMatch && cmdMatch[1]) {
      const cmd = cmdMatch[1];
      const wantsJson = pathname.endsWith('.json') || url.searchParams.get('format') === 'json';
      const params: Record<string, string> = {};
      for (const [key, value] of url.searchParams.entries()) {
        if (key !== 'format') params[key] = value;
      }
      try {
        const result = handlePreviewCommand(cmd, params, api, webUrl);
        const payload = await (result as Promise<any>);
        response.writeHead(200, { 'content-type': wantsJson ? 'application/json; charset=utf-8' : 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(wantsJson ? JSON.stringify(payload) : renderDiscordPreview(payload));
      } catch (error) {
        const message = error instanceof Error ? error.message : 'Unknown error';
        response.writeHead(502, { 'content-type': wantsJson ? 'application/json' : 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(wantsJson
          ? JSON.stringify({ error: message })
          : renderDiscordPreview({ content: `Error: ${message}`, allowedMentions: { parse: [] } }));
      }
      return;
    }

    response.writeHead(404).end();
  });
  server.listen(port, '0.0.0.0');
  return server;
}

function renderPlaygroundHTML() {
  const commandsJson = JSON.stringify(COMMANDS);
  return [
    '<!doctype html>',
    '<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">',
    '<title>PaladinsCat Preview</title><style>',
    ':root { color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; background:#1e1f22; color:#dbdee1; }',
    'html, body { margin:0; padding:0; min-height:100%; background:#313338; }',
    'main { display:flex; flex-direction:column; max-width:960px; margin:auto; min-height:100vh; padding:24px; gap:16px; }',
    'h1 { font-size:24px; margin:0; }',
    '.controls { display:flex; gap:12px; flex-wrap:wrap; align-items:flex-end; }',
    '.controls label { display:flex; flex-direction:column; gap:4px; font-size:13px; color:#949ba4; }',
    'select, input { background:#1e1f22; color:#dbdee1; border:1px solid #3f4147; border-radius:6px; padding:8px 12px; font-size:14px; outline:none; }',
    'select:focus, input:focus { border-color:#5865f2; }',
    'button { background:#5865f2; color:#fff; border:none; border-radius:6px; padding:8px 20px; font-size:14px; cursor:pointer; font-weight:600; }',
    'button:hover { background:#4752c4; }',
    'button:disabled { opacity:0.5; cursor:not-allowed; }',
    '.result { display:flex; flex-direction:column; background:#1e1f22; border-radius:8px; overflow:hidden; }',
    '.result-header { display:flex; justify-content:space-between; align-items:center; padding:12px 16px; background:#2b2d31; border-bottom:1px solid #3f4147; }',
    '.result-header h2 { font-size:14px; margin:0; }',
    '.result-header button { background:#2b2d31; border:1px solid #3f4147; font-size:12px; padding:4px 10px; }',
    '.result-frame { border:none; width:100%; height:calc(100vh - 180px); background:#1e1f22; }',
    '.loading { padding:32px; text-align:center; color:#949ba4; }',
    '.error { padding:16px; color:#ffb4ab; background:#1e1f22; border-radius:8px; }',
    '.raw-json { flex:1; overflow:auto; display:none; }',
    '.raw-json pre { background:#1e1f22; padding:12px; border-radius:6px; font-size:12px; overflow:auto; }',
    '</style></head><body><main>',
    '<h1>PaladinsCat Discord Preview</h1>',
    '<div class="controls" id="controls"></div>',
    '<div class="result" id="result" style="display:none">',
    '  <div class="result-header">',
    '    <h2 id="resultTitle">Preview</h2>',
    '    <div style="display:flex;gap:8px">',
    '      <button onclick="toggleRaw()">Toggle raw JSON</button>',
    '      <button onclick="openPreview()" style="background:#5865f2;border-color:#5865f2">Open preview</button>',
    '    </div>',
    '  </div>',
    '  <iframe class="result-frame" id="previewFrame"></iframe>',
    '  <div class="raw-json" id="rawJson"><pre id="rawJsonContent"></pre></div>',
    '</div>',
    '<div id="error" class="error" style="display:none"></div>',
    '<script>',
    'var commands = ' + commandsJson + ';',
    'var currentCommand = "";',
    'var currentUrl = "";',
    'var controls = document.getElementById("controls");',
    'var result = document.getElementById("result");',
    'var errorEl = document.getElementById("error");',
    'var frame = document.getElementById("previewFrame");',
    'var rawJsonDiv = document.getElementById("rawJson");',
    'var rawJsonContent = document.getElementById("rawJsonContent");',
    'var resultTitle = document.getElementById("resultTitle");',
    '',
    'function renderControls() {',
    '  var selectLabel = document.createElement("label");',
    '  selectLabel.innerHTML = "Command";',
    '  var select = document.createElement("select");',
    '  select.id = "cmdSelect";',
    '  commands.forEach(function(c) {',
    '    var opt = document.createElement("option");',
    '    opt.value = c.name;',
    '    opt.textContent = "/" + c.name + " - " + c.desc;',
    '    select.appendChild(opt);',
    '  });',
    '  selectLabel.appendChild(select);',
    '  controls.appendChild(selectLabel);',
    '  var paramsContainer = document.createElement("div");',
    '  paramsContainer.id = "paramsContainer";',
    '  paramsContainer.style.display = "flex";',
    '  paramsContainer.style.gap = "12px";',
    '  paramsContainer.style.flexWrap = "wrap";',
    '  paramsContainer.style.alignItems = "flex-end";',
    '  controls.appendChild(paramsContainer);',
    '  var runLabel = document.createElement("label");',
    '  runLabel.style.height = "fit-content";',
    '  var runBtn = document.createElement("button");',
    '  runBtn.id = "runBtn";',
    '  runBtn.textContent = "Preview";',
    '  runBtn.type = "button";',
    '  runLabel.appendChild(runBtn);',
    '  controls.appendChild(runLabel);',
    '  select.onchange = updateParams;',
    '  runBtn.onclick = runPreview;',
    '  updateParams();',
    '}',
    '',
    'function updateParams() {',
    '  var cmd = commands.find(function(c) { return c.name === document.getElementById("cmdSelect").value; });',
    '  var container = document.getElementById("paramsContainer");',
    '  container.innerHTML = "";',
    '  cmd && cmd.params.forEach(function(p) {',
    '    var label = document.createElement("label");',
    '    label.innerHTML = p.label;',
    '    var input = document.createElement(p.choices && p.choices.length ? "select" : "input");',
    '    input.id = p.name;',
    '    if (input.tagName === "INPUT") { input.placeholder = p.label; }',
    '    if (input.tagName === "SELECT") {',
    '      p.choices.forEach(function(choice) {',
    '        var option = document.createElement("option");',
    '        option.value = choice.value;',
    '        option.textContent = choice.label;',
    '        input.appendChild(option);',
    '      });',
    '    }',
    '    input.required = p.required;',
    '    input.style.minWidth = "180px";',
    '    label.appendChild(input);',
    '    container.appendChild(label);',
    '  });',
    '}',
    '',
    'async function runPreview() {',
    '  var cmd = document.getElementById("cmdSelect").value;',
    '  currentCommand = cmd;',
    '  errorEl.style.display = "none";',
    '  result.style.display = "none";',
    '  var cmdData = commands.find(function(c) { return c.name === cmd; });',
    '  var params = new URLSearchParams();',
    '  cmdData && cmdData.params.forEach(function(p) {',
    '    var value = document.getElementById(p.name)?.value || "";',
    '    if (value) params.set(p.name, value);',
    '  });',
    '  var url = "/preview/cmd/" + cmd + "?" + params.toString();',
    '  currentUrl = url;',
    '  frame.src = url;',
    '  resultTitle.textContent = "/" + cmd;',
    '  result.style.display = "block";',
    '  frame.style.display = "block";',
    '  rawJsonDiv.style.display = "none";',
    '  rawJsonContent.textContent = "";',
    '  frame.onload = async function() {',
    '    try {',
    '      var resp = await fetch(url + ".json");',
    '      if (!resp.ok) throw new Error("HTTP " + resp.status);',
    '      var data = await resp.json();',
    '      rawJsonContent.textContent = JSON.stringify(data, null, 2);',
    '    } catch(e) {',
    '      rawJsonContent.textContent = "Failed to fetch raw JSON: " + e.message;',
    '    }',
    '  };',
    '  frame.onerror = function() {',
    '    errorEl.style.display = "block";',
    '    errorEl.textContent = "Failed to load preview.";',
    '  };',
    '}',
    '',
    'function toggleRaw() {',
    '  rawJsonDiv.style.display = rawJsonDiv.style.display === "none" ? "block" : "none";',
    '  frame.style.display = rawJsonDiv.style.display === "block" ? "none" : "block";',
    '}',
    '',
    'function openPreview() {',
    '  window.open(currentUrl, "_blank");',
    '}',
    '',
    'renderControls();',
    '</script></main></body></html>',
  ].join('\n');
}
