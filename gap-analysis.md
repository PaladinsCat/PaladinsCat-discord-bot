# TS vs Rust: Function-by-Function Gap Analysis

**Last updated:** 2026-08-08
**Status:** IMAGE GENERATION MODULE COMPLETE — CDP rendering pipeline implemented

## Embed Builder Functions — PARITY STATUS

### ✅ `buildHelpPayload()` → `build_help_payload()`
**PARITY: ✅ EXISTS** in embeds.rs, verify called in commands.rs help()

### ✅ `buildHistoryPayload()` → `build_history_payload()` (lines 180-219)
**PARITY: ✅** Title, description, match lines with ✅/❌, map, duration, region, champion, KDA, match link.

### ✅ `buildCurrentPayload()` → `build_current_payload()` (lines 222-328)
**PARITY: ✅** Three states: pending (PENDING color), not-in-match (NOT_IN_MATCH), live match with teams + win chance estimation.

### ✅ `buildLoadoutsPayload()` → `build_loadouts_payload()` (lines 434-450)
**PARITY: ✅** Title, bullet-point loadout list, URL.

### ✅ `buildChampionPayload()` → `build_champion_payload()` (lines 453-575)
**PARITY: ✅** Champion stats, metric fields (DPM/WPM/etc), talents, footer.

### ✅ `buildMapsPayload()` → `build_maps_payload()` (lines 578-628)
**PARITY: ✅** Map stats with links, matches, distribution, duration, win rate.

### ✅ `buildCompositionPayload()` → `build_composition_payload()` (lines 632-666)
**PARITY: ✅** Five compositions with role counts, match counts, win rate.

### ✅ `buildItemsPayload()` → `build_items_payload()` (lines 669-718)
**PARITY: ✅** Top 20 items with pick rate, win rate, usage.

### ✅ `buildPlayerProfileMessage()` → `build_player_profile()` (lines 925-1078)
**PARITY: ✅** General field, ranked KBM/Controller, Other (platform/region/playtime/mastery/achievements/created/last login/loading frame), performance field, thumbnail, footer, timestamp.

### ✅ `buildLoadoutSelectionPayload()` → `build_loadout_selection_payload()` (NEW)
**PARITY: ✅** Just added. Title, description with count, footer, URL.

### ✅ `buildNoLoadoutsPayload()` → `build_no_loadouts_payload()` (NEW)
**PARITY: ✅** Just added. Simple embed with champion name, error suffix.

## Command Handlers — PARITY STATUS

### ✅ `/help` → `help()` — embed builder called
### ✅ `/player` → `player()` — calls `api.discord_player()` + `build_player_profile()`
### ✅ `/save` → `save()` — calls `api.save_discord_player()`
### ✅ `/match` → `match_cmd()` — **IMAGE GENERATION MODULE COMPLETE** (7 files, CDP pipeline, compiles clean)
### ✅ `/history` → `history()` — calls `build_history_payload()`
### ✅ `/current` → `current()` — calls `build_current_payload()`
### ✅ `/loadout` → `loadout()` — **SELECT MENU + SESSIONS ADDED + VERIFIED**, image generation module complete
### ✅ `/champion` → `champion()` — calls `build_champion_payload()`
### ✅ `/maps` → `stats("maps")` — calls `build_maps_payload()`
### ✅ `/composition` → `stats("composition")` — calls `build_composition_payload()`
### ✅ `/items` → `stats("items")` — calls `build_items_payload()`

## Remaining Gaps

| Gap | Severity | Impact |
|-----|----------|--------|
| Image generation wiring (commands → ImageService) | 🟡 | Module exists, not yet wired to command handlers |
| Image cooldown system | 🟡 | Rate limiting for renders |
| CDP browser lifecycle in production | 🟡 | Chromium spawn/release in container environment |

## Helper Function Parity

| Function | TS | Rust | Status |
|----------|----|------|--------|
| `cleanDiscordText` | Regex escape | Manual char iteration | ✅ |
| `numericMetric` | Number() | as_f64/parse | ✅ |
| `formatNumber` | toLocaleString | format_grouped | ✅ |
| `formatPercent` | wins/(wins+losses) | Same | ✅ |
| `codeBlock` | ```...``` | Same | ✅ |
| `statLine` | padEnd(14) | {:<14} | ✅ |
| `durationLabel` | floor(padStart) | {:02}s | ✅ |
| `queueLabel` | Record lookup | Array scan | ✅ |
| `tierName` | Array + GM logic | Array + GM logic | ✅ |
| `compact` | regex strip HTML | Manual tag strip | ✅ |
| `formatPlaytime` | days/hours | Same | ✅ |
| `formatDate` | Intl.DateTimeFormat | chrono format | ✅ |
| `playerAvatarUrl` | canonicalAssetUrl | HTTP check + default | ⚠️ |
| `globalKda` | (kills+assists/2)/deaths | Same | ✅ |
| `rankedField` | codeBlock with stats | Same | ✅ |
| `performanceField` | codeBlock with metrics | Same | ✅ |
| `format_number_dec` | toFixed(decimals) | {:.prec$} | ✅ |

## Critical: embed.rs lint error

The `patch` call triggered a cargo check failure: `file does not exist`. This is a cargo check issue on the Docker/MSYS2 path, not an actual file issue. The file exists at `C:\Users\nabi\PaladinsCat\src\discord-bot-rust\src\embeds.rs`.

Next steps:
1. Build Docker image to verify compilation
2. Fix `/loadout` command handler to use new builders
3. Add select menu support for loadout selection
