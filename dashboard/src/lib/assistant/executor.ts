import { calendar, trips, type TripPlan } from '$lib/api';
import { instantMinutes, localDate } from './calendar-slots';
import type { ActionResult, CalendarSlotCardData, JourneyOptionCardData } from './types';

/** Short vibration on phones that allow it; a no-op everywhere else. */
export function triggerHaptic(type: 'light' | 'medium' | 'success' = 'light'): void {
  if (typeof navigator !== 'undefined' && 'vibrate' in navigator) {
    try {
      if (type === 'light') navigator.vibrate(10);
      else if (type === 'medium') navigator.vibrate(20);
      else navigator.vibrate([12, 40, 18]);
    } catch {
      // Refused by the browser's policy; the tap still worked.
    }
  }
}

function failure(err: unknown): ActionResult {
  return { ok: false, message: err instanceof Error ? err.message : String(err) };
}

/** The local day a transit time falls on. transit sends naive local times; if one ever
 *  carries an offset or `Z`, the day is taken in the machine's zone, not UTC's. */
export function journeyDay(departure: string): string {
  return /(?:[zZ]|[+-]\d{2}:\d{2})$/.test(departure)
    ? localDate(new Date(departure))
    : departure.slice(0, 10);
}

/**
 * Pins one transit search result into the plan the operator picked.
 *
 * Only a `draft` plan, and only the one passed in: no fallback to another plan (the first
 * plan in the list can be booked or archived) and no plan created on the side. The item
 * has the shape the Travel page already writes (`routes/travel/+page.svelte`, saveJourney)
 * and `capabilities/trips/src/store.rs` DECLARED_PAYLOADS requires for `transport`.
 */
export async function executeJourneyPin(
  card: JourneyOptionCardData,
  plan: Pick<TripPlan, 'id' | 'title' | 'status'> | null,
): Promise<ActionResult> {
  if (!plan) return { ok: false, message: 'Pick a draft plan first. Nothing was saved.' };
  if (plan.status !== 'draft') {
    return { ok: false, message: `"${plan.title}" is ${plan.status}, not a draft. Nothing was saved.` };
  }
  const journey = card.journey;
  const first = journey.legs[0];
  if (!first) return { ok: false, message: 'This connection has no legs. Nothing was saved.' };
  try {
    await trips.addItem(plan.id, {
      item_type: 'transport',
      day: journeyDay(first.departure_time),
      external_id: journey.id,
      title: `${journey.start_station.name} → ${journey.end_station.name}`,
      payload: { mode: 'train', journey },
    });
    triggerHaptic('success');
    return { ok: true, message: `Pinned the connection to "${plan.title}".` };
  } catch (err) {
    return failure(err);
  }
}

/**
 * Creates the proposed block in the calendar. The instants go out exactly as the card
 * holds them: naive local "YYYY-MM-DDTHH:MM:00", which
 * `capabilities/calendar/src/date.rs::parse_instant` accepts. A trailing `Z` makes the
 * seconds field "00Z", which it rejects with a 400.
 */
export async function executeCalendarAccept(card: CalendarSlotCardData): Promise<ActionResult> {
  if (instantMinutes(card.startsAt) === null || instantMinutes(card.endsAt) === null) {
    return { ok: false, message: `Not a local wall time: ${card.startsAt} – ${card.endsAt}` };
  }
  try {
    await calendar.entries.create({
      kind: 'focus',
      title: card.title,
      starts_at: card.startsAt,
      ends_at: card.endsAt,
      commitment: 'committed',
      notes: 'Created from the Axon assistant drawer',
    });
    triggerHaptic('success');
    return { ok: true, message: `Created "${card.title}" in the calendar.` };
  } catch (err) {
    return failure(err);
  }
}

export type CardStatus =
  | { kind: 'idle' }
  | { kind: 'pending' }
  | { kind: 'applied'; message: string }
  | { kind: 'failed'; message: string };

/** What a card shows after its action settles. `applied` only on `ok: true`; a rejected
 *  promise is a failure too, so a card can never read "done" for a write that did not
 *  happen. */
export async function settleCardAction(run: () => Promise<ActionResult>): Promise<CardStatus> {
  try {
    const result = await run();
    return result.ok
      ? { kind: 'applied', message: result.message }
      : { kind: 'failed', message: result.message };
  } catch (err) {
    return { kind: 'failed', message: err instanceof Error ? err.message : String(err) };
  }
}
