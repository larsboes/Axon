import {
  scouting,
  type CalendarEntry,
  type ScoutingOpportunity,
  type ScoutingSource,
} from "../../api";
import { RESCALE, type ScoreContext, type DecisionKind } from "../decisions";
import type { CalendarSource } from "./calendar";

export interface OpportunitySource {
  opportunities: ScoutingOpportunity[];
  sources: ScoutingSource[];
}

/** An event-typed scholarship match is a known scoring artefact; its score is not usable. */
export function staleOpportunityScore(opportunity: ScoutingOpportunity): boolean {
  return (
    opportunity.opportunity_type === "event" && /scholarship/i.test(opportunity.matched_focus)
  );
}

export function safeOpportunityScore(opportunity: ScoutingOpportunity): number {
  return staleOpportunityScore(opportunity) ? 0 : opportunity.score;
}

function daysBetween(a: string, b: string): number {
  const first = new Date(`${a.slice(0, 10)}T12:00:00`);
  const second = new Date(`${b.slice(0, 10)}T12:00:00`);
  return Math.round((first.getTime() - second.getTime()) / 86_400_000);
}

function sameOpportunity(entry: CalendarEntry, opportunity: ScoutingOpportunity): boolean {
  if (!entry.payload || typeof entry.payload !== "object") return false;
  const payload = entry.payload as { opportunity_id?: unknown; url?: unknown };
  return payload.opportunity_id === opportunity.id || payload.url === opportunity.url;
}

/** The calendar kind's whole source, or empty halves while it is still loading. */
function peerCalendar(ctx: ScoreContext): CalendarSource {
  return ctx.peerSource<CalendarSource>("calendar") ?? { entries: [], contexts: [] };
}

/** Nearby planned travel lifts an opportunity; a committed clash on the day sinks it. */
export function calendarRankAdjustment(
  opportunity: ScoutingOpportunity,
  entries: readonly CalendarEntry[],
): number {
  if (!opportunity.starts_at) return 0;
  const day = opportunity.starts_at.slice(0, 10);
  let adjustment = 0;
  for (const entry of entries) {
    if (sameOpportunity(entry, opportunity)) continue;
    if (entry.commitment === "possible") continue;
    const entryDay = entry.starts_at.slice(0, 10);
    const distance = Math.abs(daysBetween(day, entryDay));
    const city = opportunity.city.trim().toLocaleLowerCase("en-GB");
    const samePlace =
      city.length > 0 && (entry.location ?? "").toLocaleLowerCase("en-GB").includes(city);
    if (samePlace && distance <= 3) adjustment += 65;
    if (entry.commitment === "committed" && entryDay === day) adjustment -= 220;
  }
  return Math.max(-220, Math.min(90, adjustment));
}

export function contextRankAdjustment(ctx: ScoreContext, opportunity: ScoutingOpportunity): number {
  if (!opportunity.starts_at) return 0;
  const day = opportunity.starts_at.slice(0, 10);
  const city = opportunity.city.trim().toLocaleLowerCase("en-GB");
  let adjustment = 0;
  for (const context of peerCalendar(ctx).contexts) {
    if (day < context.valid_from || day > context.valid_until) continue;
    const text = `${context.title} ${context.details}`.toLocaleLowerCase("en-GB");
    const nearPlanningDeadline =
      context.kind !== "planning_gap" || Math.abs(daysBetween(day, context.valid_until)) <= 3;
    if (city && text.includes(city) && nearPlanningDeadline) adjustment += 45;
    if (context.kind === "uncertainty") adjustment -= 15;
  }
  return adjustment;
}

/** One line naming what actually moved this row, rather than restating the title. */
export function opportunityRankHint(
  ctx: ScoreContext,
  opportunity: ScoutingOpportunity,
): string {
  if (staleOpportunityScore(opportunity)) return "score out of date";
  if (ctx.peerSource<CalendarSource>("calendar") === null) {
    // The dependency failed or has not answered. Say so rather than implying the
    // calendar was consulted and had nothing to add.
    return "ranked without the calendar";
  }
  const calendarAdjustment = calendarRankAdjustment(opportunity, peerCalendar(ctx).entries);
  const contextAdjustment = contextRankAdjustment(ctx, opportunity);
  if (calendarAdjustment < 0) return "conflicts with a committed event";
  if (calendarAdjustment > 0) return "fits a planned location";
  if (contextAdjustment > 0) return "fits the current planning context";
  return opportunity.matched_focus ? `matches ${opportunity.matched_focus}` : "";
}

function isPersonalized(ctx: ScoreContext, opportunity: ScoutingOpportunity): boolean {
  const lastDay = (opportunity.ends_at || opportunity.starts_at || "").slice(0, 10);
  if (lastDay && lastDay < ctx.todayKey) return false;
  if (staleOpportunityScore(opportunity)) return false;
  const { entries } = peerCalendar(ctx);
  if (entries.some((entry) => sameOpportunity(entry, opportunity))) return false;

  const linkedToPlan =
    calendarRankAdjustment(opportunity, entries) > 0 || contextRankAdjustment(ctx, opportunity) > 0;
  if (linkedToPlan) return true;

  const focus = opportunity.matched_focus.trim();
  const genericFocus = /^(events?|scholarships?) profile$/i.test(focus);
  return !genericFocus && focus.length > 0 && safeOpportunityScore(opportunity) >= 0.22;
}

/**
 * Band 600 — PRD §8.1, an opportunity.
 *
 * The only kind that reads another. Its gate drops an opportunity already in the diary,
 * and both of its rank adjustments compare it against calendar entries and planning
 * contexts, so it declares `dependsOn: ["calendar"]` and its rows are held back until
 * that kind settles. If calendar fails the adjustments are zero, the row still renders,
 * and the why-here line says the calendar was not consulted rather than implying it was.
 *
 * Old expression: `600 + 100*score + (0 <= days <= 30 ? 200 - days*4 : 0) + calendar + context`,
 * with the two adjustments bounded to [-220, 90] and unbounded-positive respectively; the
 * sum is clamped to [0, 435] before the rescale, which is the ceiling those terms reach.
 */
const opportunity: DecisionKind<OpportunitySource, ScoutingOpportunity> = {
  key: "opportunity",
  band: 600,
  label: "Scouting",
  capability: "scouting",
  view: "OpportunityRow",
  dependsOn: ["calendar"],

  load: async () => {
    const [opportunityResult, sourceResult] = await Promise.all([
      scouting.opportunities(false),
      scouting.sources(),
    ]);
    return { opportunities: opportunityResult.opportunities, sources: sourceResult.sources };
  },
  rows: (source, ctx) =>
    source.opportunities.filter((row) => row.status === "new" && isPersonalized(ctx, row)),
  id: (row) => row.id,
  title: (row) => row.title,
  urgency: (row, ctx) => {
    const days = row.starts_at ? ctx.daysUntil(row.starts_at) : 90;
    const dated = days >= 0 && days <= 30 ? 200 - days * 4 : 0;
    const raw =
      100 * safeOpportunityScore(row) +
      dated +
      calendarRankAdjustment(row, peerCalendar(ctx).entries) +
      contextRankAdjustment(ctx, row);
    return RESCALE(Math.min(435, Math.max(0, raw)), 435);
  },
  href: (row) => row.url,
  external: () => true,

  whyHere: (row, ctx) => cleanRationale(row.rationale) || opportunityRankHint(ctx, row),
  startOrDueAt: (row) => row.starts_at || null,
  candidateStatus: () => "proposed",
  // CONTRACT: scouting resolves this from the declaration in its own config -- the source
  // says what it collects, and a source that says nothing yields c1. Read, never computed.
  dataClass: (row) => row.data_class ?? null,
  processingRoute: () => null,
};

/** Drops the scorer's own debug strings and the vault-link line from a rationale. */
export function cleanRationale(value: string): string {
  if (!value || /(cosine=|hash-fallback|matched focus)/i.test(value)) return "";
  return value
    .split("\n")
    .filter((line) => !line.includes("vault link:"))
    .join(" ")
    .trim();
}

export default opportunity;
