import { calendar as calendarApi, type CalendarContext, type CalendarEntry } from "../../api";
import { entryLink } from "../../calendar/types";
import { RESCALE, type DecisionKind } from "../decisions";

export interface CalendarSource {
  entries: CalendarEntry[];
  contexts: CalendarContext[];
}

/**
 * Band 700 — PRD §8.1, a calendar entry still marked `possible`.
 *
 * Entries and contexts load together because the opportunity kind needs both and neither
 * is useful without the other. `rows()` raises only the undecided entries; the rest of
 * the source is read by `opportunity` through `ctx.peerSource`, which is why that kind
 * declares this one in `dependsOn`.
 *
 * Old expression: `700 + (0 <= days <= 30 ? 240 - days * 5 : 0) + (source === "web" ? 50 : 0)`,
 * maximum 290. A deliberately added entry outranks an imported one at the same date.
 */
const calendar: DecisionKind<CalendarSource, CalendarEntry> = {
  key: "calendar",
  band: 700,
  label: "Calendar",
  capability: "calendar",
  view: "CalendarRow",

  load: async (ctx) => {
    const [entries, contexts] = await Promise.all([
      calendarApi.entries.list(ctx.todayKey, ctx.horizonEndKey),
      calendarApi.contexts.list(ctx.todayKey, ctx.horizonEndKey),
    ]);
    return { entries, contexts };
  },
  rows: (source, ctx) =>
    source.entries.filter(
      (entry) => entry.commitment === "possible" && entry.ends_at.slice(0, 10) >= ctx.todayKey,
    ),
  id: (entry) => entry.id,
  title: (entry) => entry.title,
  urgency: (entry, ctx) => {
    const days = ctx.daysUntil(entry.starts_at);
    const dated = days >= 0 && days <= 30 ? 240 - days * 5 : 0;
    return RESCALE(dated + (entry.source === "web" ? 50 : 0), 290);
  },
  // entryLink opens the editable form in the calendar grid, and already routes through
  // link() — the base-path rule this whole page was breaking through `location.href`.
  href: (entry) => entryLink(entry),

  whyHere: (entry) => `Undecided since it was added from ${entry.source}.`,
  startOrDueAt: (entry) => entry.starts_at,
  candidateStatus: () => "proposed",
  // CalendarEntry carries no class field; only the ContentItem detail shape does.
  dataClass: () => null,
  processingRoute: () => null,
};

export default calendar;
