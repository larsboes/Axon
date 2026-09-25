import type { IntentDomain, KeywordRouting, RouteContext } from './types';

/**
 * Local keyword router. It runs in the page, matches whole words against four short cue
 * lists, and calls nothing. It is not Jev (PRD §7.1c): there is no model, no confidence and
 * no autonomy gate, so nothing it decides is ever applied without a tap.
 *
 * Whole words only. The substring matcher this replaces routed "price" to travel through
 * "ice" and "recent" to finance through "cent".
 */
const CUES: Record<Exclude<IntentDomain, 'general'>, string[]> = {
  travel: [
    'train', 'trains', 'flight', 'flights', 'trip', 'travel', 'db', 'ice', 'sncf', 'bahn',
    'station', 'airport', 'itinerary', 'destination', 'journey', 'connection', 'connections',
    'route', 'zug',
  ],
  interior: [
    'room', 'flat', 'apartment', 'furniture', 'desk', 'layout', 'layouts', 'clearance',
    'floor plan', 'living room', 'bedroom', 'hallway',
  ],
  calendar: [
    'calendar', 'meeting', 'event', 'slot', 'schedule', 'busy', 'focus', 'block',
    'appointment', 'conflict', 'overlap',
  ],
  finance: [
    'expense', 'expenses', 'spending', 'money', 'eur', 'euro', 'ledger', 'balance',
    'portfolio', 'invest', 'receipt', 'budget',
  ],
  system: [
    'system', 'systems', 'health', 'status', 'machine', 'cpu', 'ram', 'temp', 'temperature',
    'memory', 'hardware', 'power',
  ],
  feed: [
    'feed', 'reading', 'read', 'article', 'articles', 'ingest', 'comms', 'newsletter',
    'unread',
  ],
  people: [
    'duplicate', 'duplicates', 'merge', 'dedupe', 'contact', 'contacts', 'people', 'person',
    'doppelt', 'kontakte',
  ],
};

/** Lower-cased words joined by single spaces and padded, so a cue matches only on word
 *  boundaries: `" ice "` is found in `" the ice train "` and not in `" price "`. */
function wordText(text: string): string {
  const words = text.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
  return ` ${words.join(' ')} `;
}

export function matchedCues(text: string, cues: string[]): string[] {
  const padded = wordText(text);
  return cues.filter((cue) => padded.includes(` ${cue} `));
}

export function routeByKeywords(query: string, context: RouteContext): KeywordRouting {
  let best: { domain: IntentDomain; matched: string[] } = { domain: 'general', matched: [] };
  for (const domain of Object.keys(CUES) as (keyof typeof CUES)[]) {
    const matched = matchedCues(query, CUES[domain]);
    // A tie keeps the page's domain: the page is the stronger signal of the two.
    const better =
      matched.length > best.matched.length ||
      (matched.length > 0 && matched.length === best.matched.length && domain === context.domain);
    if (better) best = { domain, matched };
  }

  if (best.matched.length === 0) {
    return {
      domain: context.domain,
      matched: [],
      reason: `No cue word matched; using the page's domain (${context.label}).`,
    };
  }
  return {
    domain: best.domain,
    matched: best.matched,
    reason: `Matched ${best.matched.map((cue) => `"${cue}"`).join(', ')}.`,
  };
}
