import type { PaladinsCatApi } from './api-client.js';

/**
 * Single bot-side owner of the two reference lookups the message builders need:
 * queue id -> queue name (backend queue_types) and tier id -> tier name
 * (backend ranked_tiers).
 *
 * Per the SSoT single-source rule the backend reference tables are the only
 * source of truth. The builders previously kept their own hardcoded
 * QUEUE_LABELS / TIER_NAMES copies; those are removed and every consumer now
 * reads through this cache (R-A/R-B/R-F).
 *
 * Tier scale: ranked_tiers.tier_id is 1-27 (1=Bronze V ... 27=Grandmaster).
 * The raw player tier fields (kbm_tier / league_tier / live tier) are 0-27
 * where 0 = Unranked and 27 is the synthetic Grandmaster. So the adapter is:
 * raw 0 -> 'Unranked' (no row), raw N (1..27) -> ranked_tiers.tier_id N.
 */
export class ReferenceCache {
  private readonly queueLabels = new Map<number, string>();
  private readonly tierNames = new Map<number, string>();
  private loaded = false;

  /**
   * Fetch both reference tables from the backend. Tolerant: on failure the
   * cache keeps its current (possibly empty) state so the getters fall back
   * gracefully instead of throwing mid-render.
   */
  async load(api: PaladinsCatApi): Promise<void> {
    const [queues, tiers] = await Promise.allSettled([api.referenceQueues(), api.referenceTiers()]);
    if (queues.status === 'fulfilled') {
      this.queueLabels.clear();
      for (const row of queues.value) {
        const id = Number(row.queue_id);
        if (Number.isFinite(id) && row.queue_name) this.queueLabels.set(id, String(row.queue_name));
      }
    }
    if (tiers.status === 'fulfilled') {
      this.tierNames.clear();
      for (const row of tiers.value) {
        const id = Number(row.tier_id);
        if (Number.isFinite(id) && row.tier_name) this.tierNames.set(id, String(row.tier_name));
      }
    }
    this.loaded = queues.status === 'fulfilled' && tiers.status === 'fulfilled';
  }

  get isLoaded(): boolean {
    return this.loaded;
  }

  /**
   * Test seam: populate the maps directly without hitting the network so unit
   * tests can assert on the exact backend reference rows. Mirrors what
   * `load()` writes from the `/reference/queues` + `/reference/tiers` rows.
   */
  seed(queues: Array<{ queue_id: number; queue_name: string }>, tiers: Array<{ tier_id: number; tier_name: string }>): void {
    this.queueLabels.clear();
    for (const row of queues) this.queueLabels.set(Number(row.queue_id), String(row.queue_name));
    this.tierNames.clear();
    for (const row of tiers) this.tierNames.set(Number(row.tier_id), String(row.tier_name));
    this.loaded = true;
  }

  /** Queue id -> canonical queue name. Falls back to `Queue #N` for unknown ids. */
  queueLabel(queueId: number): string {
    const id = Number(queueId);
    if (!Number.isFinite(id) || id <= 0) return 'Unknown queue';
    return this.queueLabels.get(id) ?? `Queue #${id}`;
  }

  /**
   * Raw player tier (0-27) -> canonical tier name.
   * 0 -> 'Unranked'; 1..27 -> ranked_tiers.tier_id N.
   */
  tierName(rawTier: number): string {
    const value = Number(rawTier);
    if (!Number.isFinite(value) || value <= 0) return 'Unranked';
    const clamped = Math.min(27, Math.floor(value));
    return this.tierNames.get(clamped) ?? 'Unranked';
  }
}
