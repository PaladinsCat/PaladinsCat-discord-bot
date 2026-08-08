# TypeScript vs Rust Discord Bot — Gap Analysis

## Executive Summary

The Rust bot is a **severely stripped-down implementation** compared to TypeScript. It loses:
- **Embed color**: Uses Discord blurple (`0x5865F2`) instead of PaladinsCat accent (`0x2dd4a3`)
- **Rich formatting**: No KDA, duration, win/loss indicators, links, or codeblocks
- **URL links**: Missing embed URLs and hyperlinked names/maps/items
- **Footer text**: No footers on any embed
- **Descriptions**: Raw fields instead of formatted descriptions
- **Error messages**: Generic plain text instead of styled embeds
- **Player command**: 2 fields vs 12+ fields (General, KBM, Controller, Other, Performance)
- **Loadout**: No interactive select menu (core feature lost)
- **Match**: Plain text embed instead of rendered image

---

## 1. `/player` — Player Profile

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `{playerName}` or `{playerName} ({title})` — title shown when present | `{gamertag}` or `{name}` | ❌ Missing title display |
| **Color** | `0x2dd4a3` (PaladinsCat accent green) | `0x5865F2` (Discord blurple) | ❌ Wrong branding color |
| **URL** | `{webUrl}/players/{playerId}` | None | ❌ No deep link to web profile |
| **Thumbnail** | Player avatar URL (canonical asset or fallback) | None | ❌ No avatar |
| **Timestamp** | `profileRefresh?.refreshed_at` or `last_updated` | None | ❌ No freshness indicator |
| **Footer** | `"PaladinsCat"` | `"Gamertag: {gt}"` | ❌ Wrong footer content |
| **Fields** | 5 rich codeblock fields: General (ID, level, XP, matches, deserted, WR, KDA), Ranked KBM, Ranked Controller, Other (platform, region, playtime, mastery, achievements, account dates, loading frame), Ranked Performance (DPM, HPM, MPM, EGPM) | Only "Headroom" and "Peak Rank" as raw `to_string()` | ❌ Missing ~85% of data |
| **Data format** | `codeBlock()` format with aligned columns (`statLine` with 14-char padding) | Raw JSON values via `.to_string()` | ❌ No formatting |
| **Error state** | `PaladinsCatApiError` → specific error message; `QueueFullError` → queue message | Generic `"Failed to look up player '{name}'"` | ⚠️ Less informative |
| **Player resolution** | `playerInput()` → resolves saved Discord player or explicit name | Direct `api.discord_player(&name)` | ⚠️ No saved player lookup |
| **Markdown escape** | `escapeMarkdown()` / `cleanDiscordText()` | None | ⚠️ Vulnerable to formatting |

### TS player output example:
```
PlayerName (Custom Title)
┌─────────────────────────────────────────┐
│ General                                 │
│ ┌─────────────────────────────────────┐  │
│ │ Account ID    : 123456              │  │
│ │ Account level : 50                │  │
│ │ Total XP      : 1,234,567          │  │
│ │ Total matches : 1,892              │  │
│ │ Casual deserted: 5                │  │
│ │ Win rate      : 54.3% (1027–865)   │  │
│ │ Global KDA    : 3.45               │  │
│ └─────────────────────────────────────┘  │
│ Ranked KBM                                │
│ ┌─────────────────────────────────────┐  │
│ │ Rank        : Gold II               │  │
│ │ TP          : 67                   │  │
│ │ Win rate    : 52.1% (124–114)      │  │
│ │ Times deserted: 3                 │  │
│ └─────────────────────────────────────┘  │
│ ... (Controller, Other, Performance)     │
└─────────────────────────────────────────┘
PaladinsCat
```

### Rust player output example:
```
PlayerName
  Headroom: <raw value>
  Peak Rank: <raw value>
```

---

## 2. `/history` — Match History

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `{playerName} · Recent matches` | `Match History — {name}` | ⚠️ Different format |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **Format** | Rich description lines in embed body | Embed fields (one per match) | ❌ Different structure |
| **Per-match data** | `✅/❌ · map · duration · region · champion · KDA · [matchId](url)` | `{n}. mode: matchId` (mode as name, matchId as value) | ❌ Missing KDA, duration, win/loss, links, region, champion |
| **Win/Loss indicator** | ✅ (winner) / ❌ (loser) emoji | None | ❌ No visual indicator |
| **Duration** | `{minutes}m` formatted | None | ❌ Missing |
| **Links** | `[matchId]({webUrl}/matches/{id})` hyperlink | None | ❌ No deep links |
| **Empty state** | `"No recent matches found."` description | None (still shows title) | ⚠️ No empty message |
| **Footer** | None | None | ✓ Same |
| **Player resolution** | `playerInput()` → API resolve → `playerHistoryById(player.id, 10)` | `api.player(&name)` → get id → `api.player_history(id, 10)` | ⚠️ Different API calls |

### TS history line example:
```
✅ The Pit · 18m · NA · Cassiopeia · 12/3/8 · [284512345](https://paladinscat.com/matches/284512345)
❌ Blackpowder Bank · 12m · NA · Mako · 7/9/15 · [284512344](https://paladinscat.com/matches/284512344)
```

### Rust history field example:
```
1. Siege: 284512345
2. KBM: 284512344
```

---

## 3. `/current` — Live Match

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Pending state** | Embed: `"Live lobby loading"` (color `0xf0b232` amber), description, footer | No pending state handling | ❌ Missing |
| **Not in game** | Embed: `"Not in a live match"` (color `0x77808d` gray), description, footer | Plain text: `"{name} is not currently in a match"` | ⚠️ Embed vs plain text |
| **Title** | `{map} · Live match` | `Currently in Game — {name}` | ⚠️ Different format |
| **Color** | `0x2dd4a3` | `0x00E676` (green/IN_GAME) | ❌ Wrong color |
| **Description** | `**{queue}** · {region}\nMatch ID \`${matchId}\`` | `Map: {map} | Mode: {mode}` | ⚠️ Different format |
| **Queue labels** | `QUEUE_LABELS` map (1→Casual, 2→KBM, 4→1v1, 486→Ranked Siege, etc.) | Raw mode string | ❌ No label mapping |
| **Map cleaning** | `cleanDiscordText(String(match.map).replace(/^(?:(?:live|ranked|wip)\s+)+/i, ''))` | Raw map name | ⚠️ No prefix stripping |
| **Team display** | Two inline fields with team members: `**{champion}** · [{name}](url) · {tier} · {WR}% WR · {ELO} ELO` | None | ❌ No team roster |
| **Player highlighting** | `▸` marker for requested player | None | ❌ No player indicator |
| **Win chance** | Elo-based probability: `{team} · {percent}% win chance` | None | ❌ No analysis |
| **Player links** | Hyperlinked player names to `{webUrl}/players/{id}` | None | ❌ No deep links |
| **Footer** | `"Estimate blends queue ELO with global win rate · ▸ marks the requested player · Live lobby snapshot"` | None | ❌ No footer |
| **Timestamp** | `match.detected_at` ISO timestamp | None | ❌ No timestamp |

---

## 4. `/loadout` — Champion Loadouts

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Flow** | Select menu with options → user picks → renders image attachment | Static embed listing 5 loadouts | ❌ No interactivity |
| **No loadouts** | Embed: `{playerName} · {championName}`, description with refresh status | Plain text: `"No {champion} loadouts found for {name}"` | ⚠️ Embed vs plain text |
| **Title** | `{playerName} · {championName}` | `{champion} Loadouts — {name}` | ⚠️ Different order |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **URL** | `{webUrl}/players/{playerId}/loadouts` | None | ❌ No deep link |
| **Description** | `"Choose one of **{count}** saved loadout(s) below..."` | None | ❌ Missing |
| **Footer** | `"Served from the PaladinsCat database."` or refresh message | None | ❌ Missing |
| **Loadout display** | Interactive StringSelectMenuBuilder with 25 options (card point totals) | Embed fields: loadout name → "{n} cards" for 5 loadouts | ❌ Different format |
| **Selection** | Session token, 5min TTL, user validation, image render | None | ❌ Core feature missing |
| **Player resolution** | `playerInput()` + `findPlayerChampionLoadouts()` with refresh | Direct `api.player()` → `api.loadouts()` + manual filter | ⚠️ Different API path |

---

## 5. `/champion` — Champion Stats

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `{name} · Ranked performance` | `{champion}` | ❌ Missing subtitle |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **URL** | `{webUrl}/champions/{name}` (lowercase) | None | ❌ No deep link |
| **Description** | `"**{lobbyLabel}** · Served from the PaladinsCat champion database."` | None | ❌ Missing |
| **Fields** | Class, Average lobby tier, Ranked record, DPM, WPM, APM, CPM, HPM, SPM, KDA, Most played talents | Win Rate, Pick Rate, Games | ❌ Missing ~80% of fields |
| **Tier display** | `**{TierName}**\n{average} average` with TIER_NAMES array (Bronze V–Grandmaster) | None | ❌ Missing tier names |
| **Record format** | `"**{WR}%** win rate\n{W} W · {L} L\n{total} total plays"` | None | ❌ Missing |
| **Metrics** | 7 inline fields with P10-P90 ranges: `"**{avg}**\nP10–P90 {p10}–{p90}"` | None | ❌ Missing percentile data |
| **Talent data** | Top 3 talents sorted by plays, with WR, pick rate, play count | None | ❌ Missing |
| **Footer** | `"Lobby filters use the ranked match database; global is the default."` | None | ❌ Missing |
| **Lobby scope** | `rankedLobbyScope()` with labeled choices (Global, KBM, Siege, etc.) | Raw string, no validation | ⚠️ No scope validation |

---

## 6. `/maps` — Map Statistics

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `"Ranked map statistics"` | `"Ranked Map Stats"` | ⚠️ Different wording |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **URL** | `{webUrl}/game/maps` | None | ❌ No deep link |
| **Description** | Rich lines per map with hyperlinks, match counts, pool %, avg duration, win rate | None (uses fields instead) | ❌ Missing |
| **Map links** | `[mapName]({webUrl}/game/maps/{name})` | None | ❌ No deep links |
| **Format** | `**[MapName](url)** · {matches} matches · {pool}% of pool · {duration} avg · {WR}% WR` | Fields: `{map}` → `{games}` (just name + count) | ❌ Missing pool %, duration, WR |
| **Map name cleaning** | `cleanDiscordText(name.replace(/^Ranked\s+/i, ''))` | Raw map name | ⚠️ No prefix stripping |
| **Count** | 100 maps requested, description truncated at 4000 chars | 10 maps only | ⚠️ 10x less data |
| **Footer** | `"PaladinsCat ranked match database · Ordered by matches played"` | None | ❌ Missing |

---

## 7. `/composition` — Team Compositions

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `"Top ranked team compositions"` | `"Top Compositions"` | ⚠️ Shortened |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **URL** | `{webUrl}/game/compositions` | None | ❌ No deep link |
| **Description** | `"Most-played global ranked role lineups."` or empty state | None | ❌ Missing |
| **Field names** | `"{n}. {FL} Frontline · {D} Damage · {F} Flank · {S} Support"` | Raw champion string (JSON?) | ❌ No role breakdown |
| **Field values** | `"{matches} matches · {WR}% win rate"` | `"{games}"` (count only) | ❌ Missing win rate |
| **Count** | 5 compositions (top 5) | 10 compositions | ⚠️ Different count |
| **Footer** | `"Top five by matches played · PaladinsCat ranked match database"` | None | ❌ Missing |

---

## 8. `/items` — Item Statistics

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `"Ranked item statistics"` | `"Item Stats"` | ⚠️ Different wording |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **URL** | `{webUrl}/game/items` | None | ❌ No deep link |
| **Description** | `"**{lobbyLabel}**\n{lines}"` | None | ❌ Missing |
| **Format** | `**{n}. [itemName](url)** · {pick}% pick · {WR}% WR · {uses} uses` | Fields: `{item}` → `{pick}` | ❌ Missing WR, uses, rank |
| **Item links** | `[item_name]({webUrl}/game/items/{item_id})` | None | ❌ No deep links |
| **Lobby scope** | `rankedLobbyScope()` with labeled choices | None (no lobby option) | ❌ Missing lobby filter |
| **Count** | 20 items | 10 items | ⚠️ Half the data |
| **Footer** | `"Top twenty by usage · Global ranked lobbies are the default"` | None | ❌ Missing |

---

## 9. `/help` — Help Command

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Title** | `"PaladinsCat commands"` | `"PaladinsCat Bot Commands"` | ⚠️ Slight difference |
| **Color** | `0x2dd4a3` | `0x5865F2` | ❌ Wrong color |
| **Format** | Markdown with backticks, descriptions for each command | Code block with ` — ` separator, no descriptions | ⚠️ Less informative |
| **Commands listed** | 10 commands + note about player options | 9 commands (missing `/save` in help text, though implemented) | ⚠️ Missing `/save` help |
| **Ephemeral** | `ephemeral: true` | Not ephemeral (visible to channel) | ⚠️ Different visibility |

---

## 10. `/match` — Match Result

| Aspect | TypeScript | Rust | Gap |
|--------|------------|------|-----|
| **Output** | Rendered PNG image attachment via RenderService | Plain embed with mode, duration, map fields | ❌ Completely different output type |
| **Title** | (N/A — image attachment) | `"Match Result"` | N/A |
| **Color** | (N/A) | `0x00E676` (green/VICTORY) | ⚠️ Static color (not win/loss aware) |
| **URL** | `{webUrl}/matches/{id}` as message content | None | ❌ Missing deep link |
| **Validation** | Regex `^\d{6,20}$` on match ID | None | ⚠️ No validation |
| **Cooldown** | 10-second per-user cooldown via `claimImageCooldown` | None | ⚠️ No rate limiting |
| **Error** | Specific error with timing info | Generic `"Match '{id}' not found"` | ⚠️ Less informative |

---

## Structural Gaps Summary

### Missing Architecture in Rust

| Feature | TypeScript | Rust |
|---------|------------|------|
| **Message builder pattern** | Dedicated `message-builders.ts` with 10+ exported builders | Inline embed building per command |
| **Embed payload** | `assertDiscordMessage()` with `allowedMentions: { parse: [] }` | Raw `InteractionResponseData` |
| **Data formatting** | `cleanDiscordText()`, `formattedNumber()`, `durationLabel()`, `statLine()`, `codeBlock()` | `.to_string()` everywhere |
| **Player resolution** | `playerInput()` → saved player lookup → fallback to explicit | Direct API call with raw name |
| **Loadout session** | UUID session tokens, 5-min TTL, user validation, select menus | None |
| **Image rendering** | `RenderService` → PNG attachments | None |
| **Cooldown system** | Per-user 10s cooldown for images | None |
| **Autocomplete** | Cached champion list (1hr TTL), scoring-based ranking | API call per autocomplete, simpler sorting |
| **Error handling** | `PaladinsCatApiError`, `QueueFullError` with specific messages | Generic `Err` messages |

### Color Consistency

| Constant | TypeScript | Rust |
|----------|------------|------|
| **Default/Accent** | `0x2dd4a3` (green) | `0x5865F2` (blurple) |
| **Win** | Not used in embeds (✅ emoji) | `0x00E676` (green) |
| **Loss** | Not used in embeds (❌ emoji) | `0xFF1744` (red) |
| **Pending** | `0xf0b232` (amber) | None |
| **Not in game** | `0x77808d` (gray) | None |

### Hyperlink Coverage

| Link Type | TypeScript | Rust |
|-----------|------------|------|
| Player profile | ✅ `{webUrl}/players/{id}` | ❌ |
| Match result | ✅ `{webUrl}/matches/{id}` | ❌ |
| Loadout page | ✅ `{webUrl}/players/{id}/loadouts` | ❌ |
| Champion page | ✅ `{webUrl}/champions/{name}` | ❌ |
| Map page | ✅ `{webUrl}/game/maps/{name}` | ❌ |
| Item page | ✅ `{webUrl}/game/items/{id}` | ❌ |
| Composition page | ✅ `{webUrl}/game/compositions` | ❌ |

### Markdown Safety

| Feature | TypeScript | Rust |
|---------|------------|------|
| Markdown escaping | ✅ `escapeMarkdown()` + `cleanDiscordText()` regex | ❌ Raw strings |
| Code blocks | ✅ ` ``` ` for stat fields | ❌ None |
| Bold formatting | ✅ `**text**` for emphasis | ❌ None |
| Duration formatting | ✅ `{min}m` or `{min}m {sec}s` | ❌ None |
| Number formatting | ✅ `toLocaleString()` with commas | ❌ Raw numbers |
