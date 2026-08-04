import type {
  CalendarCandidateVerdict,
  PlaceRef,
  ScoutingOpportunity,
  TripPlan,
} from "../api";

/** A destination is an area, not a pin-sized venue. This radius decides only
 * whether an event belongs to an existing plan; it is not a relevance weight
 * and is deliberately separate from Scouting's private home-radius policy. */
export const DESTINATION_MATCH_RADIUS_KM = 75;

export type TravelCandidateState =
  | "matching_trip"
  | "free_window"
  | "needs_travel_day"
  | "conflicts"
  | "date_unresolved"
  | "calendar_unavailable";

export interface TravelPlanMatch {
  plan: TripPlan;
  destination: PlaceRef;
  distance_km: number;
}

export interface TravelCandidateAssessment {
  opportunity: ScoutingOpportunity;
  state: TravelCandidateState;
  plan_match: TravelPlanMatch | null;
  calendar_verdict: CalendarCandidateVerdict | null;
  destination_reason: string;
  reason: string;
}

interface Coordinate {
  latitude: number;
  longitude: number;
}

function coordinateOf(value: {
  latitude?: number | null;
  longitude?: number | null;
}): Coordinate | null {
  const latitude = value.latitude;
  const longitude = value.longitude;
  if (
    typeof latitude !== "number" ||
    typeof longitude !== "number" ||
    !Number.isFinite(latitude) ||
    !Number.isFinite(longitude) ||
    latitude < -90 ||
    latitude > 90 ||
    longitude < -180 ||
    longitude > 180
  ) {
    return null;
  }
  return { latitude, longitude };
}

export function haversineKm(from: Coordinate, to: Coordinate): number {
  const radians = (degrees: number) => (degrees * Math.PI) / 180;
  const fromLatitude = radians(from.latitude);
  const toLatitude = radians(to.latitude);
  const latitudeDelta = toLatitude - fromLatitude;
  const longitudeDelta = radians(to.longitude - from.longitude);
  const arc =
    Math.sin(latitudeDelta / 2) ** 2 +
    Math.cos(fromLatitude) * Math.cos(toLatitude) * Math.sin(longitudeDelta / 2) ** 2;
  return 2 * 6_371.0088 * Math.atan2(Math.sqrt(arc), Math.sqrt(1 - arc));
}

function eventDay(opportunity: ScoutingOpportunity): string | null {
  const day = opportunity.starts_at?.slice(0, 10) ?? "";
  return /^\d{4}-\d{2}-\d{2}$/.test(day) ? day : null;
}

function travelCandidates(
  opportunities: ScoutingOpportunity[],
  today: string,
): ScoutingOpportunity[] {
  return opportunities.filter(
    (opportunity) => {
      if (
        opportunity.status === "dismissed" ||
        opportunity.event_route?.route !== "travel_candidate"
      ) {
        return false;
      }
      const startsOn = eventDay(opportunity);
      const endsOn = /^\d{4}-\d{2}-\d{2}$/.test(opportunity.ends_at?.slice(0, 10) ?? "")
        ? opportunity.ends_at.slice(0, 10)
        : startsOn;
      return !endsOn || endsOn >= today;
    },
  );
}

export function findDestinationMatch(
  opportunity: ScoutingOpportunity,
  plans: TripPlan[],
  today: string,
  radiusKm = DESTINATION_MATCH_RADIUS_KM,
): TravelPlanMatch | null {
  const eventCoordinate = coordinateOf(opportunity);
  const day = eventDay(opportunity);
  if (!eventCoordinate || !day || !Number.isFinite(radiusKm) || radiusKm <= 0) return null;

  const matches: TravelPlanMatch[] = [];
  for (const plan of plans) {
    if (plan.date_end < today || day < plan.date_start || day > plan.date_end) continue;
    for (const destination of plan.destinations) {
      const destinationCoordinate = coordinateOf(destination);
      if (!destinationCoordinate) continue;
      const distance = haversineKm(eventCoordinate, destinationCoordinate);
      if (distance <= radiusKm) {
        matches.push({
          plan,
          destination,
          distance_km: Math.round(distance * 10) / 10,
        });
      }
    }
  }

  return (
    matches.sort(
      (left, right) =>
        left.distance_km - right.distance_km ||
        left.plan.date_start.localeCompare(right.plan.date_start) ||
        left.plan.id.localeCompare(right.plan.id) ||
        left.destination.id.localeCompare(right.destination.id),
    )[0] ?? null
  );
}

function destinationReason(
  opportunity: ScoutingOpportunity,
  plans: TripPlan[],
  match: TravelPlanMatch | null,
): string {
  if (match) {
    return `${match.distance_km.toFixed(1)} km from ${match.destination.name} during ${match.plan.title}.`;
  }
  if (!coordinateOf(opportunity)) {
    return "Destination match unavailable: the event has no complete coordinate.";
  }
  if (plans.length === 0) return "No upcoming trip exists to match against.";
  return `No dated upcoming destination is within ${DESTINATION_MATCH_RADIUS_KM} km.`;
}

export function calendarCandidatesFor(
  opportunities: ScoutingOpportunity[],
  plans: TripPlan[],
  today: string,
): Array<{ id: string; starts_at: string; ends_at: string | null }> {
  return travelCandidates(opportunities, today).flatMap((opportunity) => {
    if (findDestinationMatch(opportunity, plans, today) || !eventDay(opportunity)) return [];
    return [
      {
        id: opportunity.id,
        starts_at: opportunity.starts_at,
        ends_at: opportunity.ends_at?.trim() || null,
      },
    ];
  });
}

export function assessTravelCandidates(
  opportunities: ScoutingOpportunity[],
  plans: TripPlan[],
  verdicts: ReadonlyMap<string, CalendarCandidateVerdict>,
  today: string,
  calendarAvailable: boolean,
): TravelCandidateAssessment[] {
  const priority: Record<TravelCandidateState, number> = {
    matching_trip: 0,
    free_window: 1,
    needs_travel_day: 2,
    calendar_unavailable: 3,
    date_unresolved: 4,
    conflicts: 5,
  };

  return travelCandidates(opportunities, today)
    .map((opportunity): TravelCandidateAssessment => {
      const planMatch = findDestinationMatch(opportunity, plans, today);
      const destination = destinationReason(opportunity, plans, planMatch);
      if (planMatch) {
        return {
          opportunity,
          state: "matching_trip",
          plan_match: planMatch,
          calendar_verdict: null,
          destination_reason: destination,
          reason: `Matches an existing trip: ${destination}`,
        };
      }
      if (!eventDay(opportunity)) {
        return {
          opportunity,
          state: "date_unresolved",
          plan_match: null,
          calendar_verdict: null,
          destination_reason: destination,
          reason: `${destination} Calendar check unavailable: the event has no usable date.`,
        };
      }

      const calendarVerdict = verdicts.get(opportunity.id) ?? null;
      if (!calendarAvailable || !calendarVerdict) {
        return {
          opportunity,
          state: "calendar_unavailable",
          plan_match: null,
          calendar_verdict: null,
          destination_reason: destination,
          reason: `${destination} Calendar feasibility is unavailable.`,
        };
      }

      const state: TravelCandidateState =
        calendarVerdict.verdict === "free"
          ? "free_window"
          : calendarVerdict.verdict === "needs-travel-day"
            ? "needs_travel_day"
            : "conflicts";
      const calendarReason =
        state === "free_window"
          ? "Calendar reports a free window."
          : state === "needs_travel_day"
            ? "Calendar reports a movable conflict: a travel or remote day is needed."
            : "Calendar reports a hard conflict; do not plan this event now.";
      return {
        opportunity,
        state,
        plan_match: null,
        calendar_verdict: calendarVerdict,
        destination_reason: destination,
        reason: `${destination} ${calendarReason}`,
      };
    })
    .sort(
      (left, right) =>
        priority[left.state] - priority[right.state] ||
        (eventDay(left.opportunity) ?? "9999-99-99").localeCompare(
          eventDay(right.opportunity) ?? "9999-99-99",
        ) ||
        left.opportunity.title.localeCompare(right.opportunity.title) ||
        left.opportunity.id.localeCompare(right.opportunity.id),
    );
}
