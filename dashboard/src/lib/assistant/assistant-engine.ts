import { axonStatus, calendar, interior, transit, trips, type IntentDraft, type Journey } from '$lib/api';
import { addDays, findFreeSlots, localDate, unreadableEntries } from './calendar-slots';
import { matchedCues, routeByKeywords } from './keyword-router';
import type { ActionCard, AssistantMessage, IntentDomain, RouteContext } from './types';

/**
 * Answers a drawer prompt from the capabilities on this machine, and from nothing else.
 *
 * Every number, time and name in a reply or on a card comes from a capability response.
 * A field the capability did not determine is shown as unresolved; a capability that did
 * not answer is named, and the reply stops there. There is no offline fallback with
 * sample data: the drawer says it cannot answer instead (operator ruling 2026-09-24).
 *
 * Nothing here leaves loopback: trips, transit, calendar and interior are local services.
 * The prompt goes to trips' intent parser as typed; privacy boundaries for anything that
 * does leave the machine live in Rust (`libs/pseudonymize`, PRD Q112), not in the page.
 */

interface Reply {
  content: string;
  cards?: ActionCard[];
}

/** Departure time sent to transit when the sentence names none. Stated in the reply. */
export const DEFAULT_DEPARTURE = '08:00';
/** The window a free block is searched in. Stated in the reply. */
export const FOCUS_WINDOW = { start: '09:00', end: '18:00' } as const;
/** Block length when the sentence names none. Stated in the reply. */
export const DEFAULT_BLOCK_MINUTES = 60;
const MAX_JOURNEY_CARDS = 3;

function reason(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function didNotAnswer(capability: string, err: unknown, consequence: string): Reply {
  return { content: `${capability} did not answer: ${reason(err)}. ${consequence}` };
}

export class AssistantEngine {
  async processQuery(
    prompt: string,
    context: RouteContext,
    now: Date = new Date(),
  ): Promise<AssistantMessage> {
    const routing = routeByKeywords(prompt, context);
    const { content, cards } = await this.answer(prompt, routing.domain, now);
    return {
      id: `msg-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      role: 'assistant',
      content,
      timestamp: new Date().toISOString(),
      routing,
      cards,
    };
  }

  private answer(prompt: string, domain: IntentDomain, now: Date): Promise<Reply> {
    switch (domain) {
      case 'travel':
        return this.travel(prompt);
      case 'calendar':
        return this.calendar(prompt, now);
      case 'interior':
        return this.interior();
      case 'finance':
        return Promise.resolve({
          content: 'The drawer cannot answer finance questions yet. Open Finance for balances and the review queue.',
        });
      default:
        return Promise.resolve({
          content:
            'The drawer can search train connections (trips + transit), find a free calendar block (calendar) and list interior layouts (interior). It cannot answer other questions yet.',
        });
    }
  }

  private async travel(prompt: string): Promise<Reply> {
    let drafted: IntentDraft;
    try {
      drafted = await trips.draftIntent(prompt);
    } catch (err) {
      return didNotAnswer('trips', err, 'No connection was searched.');
    }

    const draft = drafted.draft;
    const origin = draft.origin?.name ?? null;
    const destination = draft.destinations[0]?.name ?? null;
    const date = draft.date_start;
    const lines = [
      `trips read: from ${origin ?? '(not stated)'} to ${destination ?? '(unresolved)'} on ${date ?? '(unresolved)'}.`,
    ];
    if (drafted.unresolved.length > 0) lines.push(`Unresolved: ${drafted.unresolved.join(', ')}.`);
    if (drafted.assumptions.length > 0) lines.push(`trips assumed: ${drafted.assumptions.join('; ')}.`);

    if (!destination || !date) {
      lines.push('Not searched: a destination and a date are both needed.');
      return { content: lines.join('\n') };
    }
    const modes = draft.transport_modes;
    if (modes.length > 0 && !modes.includes('train')) {
      lines.push(`Not searched: transit searches rail only, and the sentence asked for ${modes.join(', ')}.`);
      return { content: lines.join('\n') };
    }

    // transit is on-demand. Start it before the search, as /travel/connections does on
    // mount (operator ruling 2026-09-24). A failed start is not fatal here: the search
    // below reports the real error.
    await axonStatus.start('transit').catch(() => undefined);

    let journeys: Journey[];
    try {
      journeys = await transit.search(origin, destination, `${date}T${DEFAULT_DEPARTURE}:00`);
    } catch (err) {
      lines.push(`transit did not answer: ${reason(err)}. No connection is shown.`);
      return { content: lines.join('\n') };
    }

    const when = `departures from ${DEFAULT_DEPARTURE} on ${date} (the sentence named no time)`;
    if (origin === null) lines.push("No origin stated: transit started from the profile's first home station.");
    if (journeys.length === 0) {
      lines.push(`transit found no connection for ${when}.`);
      return { content: lines.join('\n') };
    }
    const shown = journeys.slice(0, MAX_JOURNEY_CARDS);
    lines.push(
      `transit found ${journeys.length} connection${journeys.length === 1 ? '' : 's'} for ${when}; the first ${shown.length} follow.`,
    );
    return {
      content: lines.join('\n'),
      cards: shown.map((journey) => ({ type: 'journey_option', data: { journey } })),
    };
  }

  private async calendar(prompt: string, now: Date): Promise<Reply> {
    if (matchedCues(prompt, ['focus', 'slot', 'block', 'free', 'time']).length === 0) {
      return {
        content: 'The drawer can find a free block in the calendar ("find a 2-hour focus block tomorrow"). It cannot answer other calendar questions yet.',
      };
    }

    const today = localDate(now);
    const explicit = /\b(\d{4}-\d{2}-\d{2})\b/.exec(prompt)?.[1];
    const date = explicit
      ? explicit
      : matchedCues(prompt, ['today']).length > 0
        ? today
        : matchedCues(prompt, ['tomorrow']).length > 0
          ? addDays(today, 1)
          : null;
    if (!date) {
      return { content: 'Which day? Say "today", "tomorrow" or a date as YYYY-MM-DD. No slot was searched.' };
    }

    const hours = /(\d+(?:[.,]\d+)?)\s*-?\s*(?:hours?|h)\b/i.exec(prompt);
    const minutes = /(\d+)\s*-?\s*(?:minutes?|mins?)\b/i.exec(prompt);
    const stated = hours
      ? Math.round(Number(hours[1].replace(',', '.')) * 60)
      : minutes
        ? Number(minutes[1])
        : null;
    const duration = stated ?? DEFAULT_BLOCK_MINUTES;

    let entries;
    try {
      // `to` is exclusive in the calendar's day window
      // (`capabilities/calendar/src/store.rs::day_window_bounds`), so one day is [date, date+1).
      entries = await calendar.entries.list(date, addDays(date, 1));
    } catch (err) {
      return didNotAnswer('calendar', err, 'No slot was proposed.');
    }

    const lines: string[] = [];
    if (stated === null) lines.push(`No length stated; searched for ${duration} minutes.`);
    const unreadable = unreadableEntries(entries);
    if (unreadable.length > 0) {
      lines.push(
        `${unreadable.length} entr${unreadable.length === 1 ? 'y has' : 'ies have'} times the drawer cannot read and ${unreadable.length === 1 ? 'was' : 'were'} not checked: ${unreadable.map((e) => `"${e.title}"`).join(', ')}.`,
      );
    }

    const free = findFreeSlots(
      { date, windowStart: FOCUS_WINDOW.start, windowEnd: FOCUS_WINDOW.end, durationMinutes: duration },
      entries,
    );
    const window = `between ${FOCUS_WINDOW.start} and ${FOCUS_WINDOW.end} on ${date}`;
    if (free.length === 0) {
      lines.unshift(`No free ${duration}-minute block ${window}. The calendar holds ${entries.length} entr${entries.length === 1 ? 'y' : 'ies'} that day.`);
      return { content: lines.join('\n') };
    }
    lines.unshift(
      `Checked ${entries.length} calendar entr${entries.length === 1 ? 'y' : 'ies'} ${window}. Free ${duration}-minute block${free.length === 1 ? '' : 's'}:`,
    );
    return {
      content: lines.join('\n'),
      cards: free.map((slot) => ({
        type: 'calendar_slot',
        data: { title: 'Focus block', startsAt: slot.startsAt, endsAt: slot.endsAt },
      })),
    };
  }

  private async interior(): Promise<Reply> {
    try {
      const layouts = await interior.layouts();
      if (layouts.length === 0) {
        return { content: 'interior holds no layouts yet. The drawer cannot evaluate or change layouts.' };
      }
      const list = layouts
        .map((l) => `- ${l.name}: ${l.pass ? 'passes' : 'fails'} the check (${l.hard} hard, ${l.soft} soft rule violations)`)
        .join('\n');
      return {
        content: `interior holds ${layouts.length} layout${layouts.length === 1 ? '' : 's'}:\n${list}\nThe drawer cannot evaluate or change layouts yet; open Interior for that.`,
      };
    } catch (err) {
      return didNotAnswer('interior', err, 'No layout is shown.');
    }
  }
}

export const assistantEngine = new AssistantEngine();
