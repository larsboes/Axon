import { comms, type FeedEntry } from "../../api";
import { link } from "../../nav";
import { RESCALE, type DecisionKind } from "../decisions";

const TWENTY_FOUR_HOURS = 86_400_000;

/** The score /feed itself orders by: the factorised evaluation first, the legacy row second. */
export const feedScore = (entry: FeedEntry): number =>
  entry.evaluation?.overall_score ?? entry.relevance?.score ?? 0;

/**
 * Band 500 — PRD §8.1, a feed item.
 *
 * The reading lane, not the commitment lane. Reading never expires, and interleaving 53
 * articles with three decisions by score was the original failure this page was rebuilt
 * to fix, so this kind is ranked but shown apart.
 *
 * Old expression: `500 + 100 * relevance.score + (age < 24h ? 40 : 0)`, maximum 140. The
 * one change is the score it reads: `evaluation.overall_score` first, matching /feed's own
 * ordering. An item with a factorised evaluation and no legacy relevance row used to
 * score 0 here while /feed put it at the top.
 */
const feed: DecisionKind<FeedEntry[], FeedEntry> = {
  key: "feed",
  band: 500,
  label: "Feed",
  capability: "comms",
  lane: "reading",
  view: "FeedRow",

  load: () => comms.feed({ days: 30 }),
  rows: (entries) => entries.filter((entry) => entry.status === "new"),
  id: (entry) => entry.id,
  title: (entry) => entry.title ?? entry.url,
  urgency: (entry, ctx) => {
    const recent = ctx.nowMs - new Date(entry.created_at).getTime() < TWENTY_FOUR_HOURS ? 40 : 0;
    return RESCALE(100 * feedScore(entry) + recent, 140);
  },
  href: (entry) => link(`/feed/${encodeURIComponent(entry.id)}`),

  whyHere: (entry) => {
    const factors = entry.evaluation?.factors ?? [];
    if (factors.length > 0) {
      // The two factors that actually carried the score, named. The explanation string
      // is a paragraph and belongs on the entry page, not on a ladder row.
      const top = [...factors].sort((a, b) => b.score * b.weight - a.score * a.weight).slice(0, 2);
      return top.map((factor) => factor.label).join(" and ");
    }
    return entry.relevance ? `Matches ${entry.relevance.profile_label}.` : "";
  },
  startOrDueAt: (entry) => entry.created_at,
  candidateStatus: () => "proposed",
  // FeedEntry carries no class field; the ContentItem detail shape does. Until comms
  // publishes one on the list contract there is nothing here that is not a guess.
  dataClass: () => null,
  processingRoute: () => null,
  scoreFactors: (entry) =>
    (entry.evaluation?.factors ?? []).map((factor) => ({
      label: factor.label,
      score: factor.score,
    })),
};

export default feed;
