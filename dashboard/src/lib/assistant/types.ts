import type { Journey } from '$lib/api';

export type IntentDomain = 'travel' | 'interior' | 'calendar' | 'finance' | 'general';

/** How the local keyword router picked a domain. In-page word matching, no model. */
export interface KeywordRouting {
  domain: IntentDomain;
  /** The whole-word cues that matched, for the tooltip. Empty when the route decided. */
  matched: string[];
  reason: string;
}

/** A connection exactly as transit's `/api/search` returned it. Nothing on this card is
 *  derived here except the display strings. */
export interface JourneyOptionCardData {
  journey: Journey;
}

/** A proposed calendar block. `startsAt`/`endsAt` are naive local wall times
 *  ("YYYY-MM-DDTHH:MM:00"), the format `capabilities/calendar/src/date.rs::parse_instant`
 *  accepts; an offset or a trailing `Z` is a 400 there. */
export interface CalendarSlotCardData {
  title: string;
  startsAt: string;
  endsAt: string;
}

export type ActionCard =
  | { type: 'journey_option'; data: JourneyOptionCardData }
  | { type: 'calendar_slot'; data: CalendarSlotCardData };

export interface AssistantMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  routing?: KeywordRouting;
  cards?: ActionCard[];
}

export interface ActionResult {
  ok: boolean;
  message: string;
}

export interface RouteContext {
  pathname: string;
  domain: IntentDomain;
  label: string;
  contextSummary: string;
  quickPrompts: string[];
}
