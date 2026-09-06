import { comms, type TriageItem } from "../../api";
import { link } from "../../nav";
import { MAX_URGENCY, type DecisionKind } from "../decisions";

/**
 * The model rung's score, in basis points, once mail-triage publishes it.
 *
 * Narrowed here rather than read from the shared `TriageItem` on purpose: that edit
 * belongs to another pass, and declaring the field locally means neither this file nor
 * theirs has to land first. When it arrives it becomes the row's whole urgency.
 */
export type MailRow = TriageItem & { score_bp?: number };

const FORTY_EIGHT_HOURS = 172_800_000;

/**
 * Band 550 — PRD §8.1, mail.
 *
 * Only `aktiv` mail reaches the ladder at all. The other six categories are the ones the
 * rules already decided about — a receipt or a newsletter has no call left to make — and
 * putting the whole inbox here would repeat the mistake the reading lane exists to fix,
 * with 25 proposals instead of 53 articles.
 *
 * Urgency is NOT a rescale of the old expression, and that is deliberate. The only term
 * was a 48-hour recency nudge worth 30 points in a band where nothing else varied;
 * rescaling 30 by its own maximum would put every recent mail at the top of the band.
 * 120 keeps a nudge a nudge — 12% of the range — and it is superseded entirely the moment
 * `score_bp` arrives.
 */
const mail: DecisionKind<TriageItem[], MailRow> = {
  key: "mail",
  band: 550,
  label: "Mail",
  capability: "comms",
  view: "MailRow",

  load: () => comms.triage("proposed"),
  rows: (items) =>
    (items as MailRow[]).filter((item) => item.status === "proposed" && item.stream === "aktiv"),
  id: (item) => item.id,
  title: (item) => item.subject ?? "(no subject)",
  urgency: (item, ctx) => {
    if (item.score_bp !== undefined && item.score_bp !== null) {
      // 0..10000 bp over 0..999: 10000 / 10.01 is 999.0.
      return Math.min(MAX_URGENCY, Math.max(0, Math.round(item.score_bp / 10.01)));
    }
    // `ctx.nowMs`, not the wall clock. Every other kind ranks against the context's own
    // day, which is recomputed at local midnight; reading `Date.now()` here made this the
    // one kind whose rank could not be reproduced from a context, and forced its test to
    // build a fixture relative to the moment the test ran.
    const age = item.internal_date
      ? ctx.nowMs - new Date(item.internal_date).getTime()
      : Infinity;
    return age < FORTY_EIGHT_HOURS ? 120 : 0;
  },
  // The entry route resolves an item from its source alone, so mail opens the same reader
  // feed does — with the Gmail actions its extension adds.
  href: (item) => link(`/feed/${encodeURIComponent(item.id)}?source=mail`),

  // The rung's own words, never a restatement of the subject the row already shows.
  // PRD:227: agents rank AND explain.
  whyHere: (item) => item.rationale,
  startOrDueAt: (item) => item.internal_date,
  candidateStatus: () => "proposed",
  // The only kind with a real answer: comms publishes a class on every triage item.
  dataClass: (item) => item.data_class,
  processingRoute: () => null,
};

export default mail;
