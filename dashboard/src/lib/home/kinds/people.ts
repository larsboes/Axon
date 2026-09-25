import { entities, type Entity, type TripPlan } from "../../api";
import { link } from "../../nav";
import { RESCALE, type DecisionKind } from "../decisions";

export interface PersonDecisionRow {
  person: Entity;
  reason: "birthday" | "trip_proximity" | "cadence";
  title: string;
  detail: string;
  daysUntil: number | null;
  targetDate: string | null;
  city?: string;
}

export interface PeopleSource {
  people: Entity[];
}

/**
 * Band 540 — PRD §8.1, person contact frequency / relationship radar.
 *
 * Raises relationship touchpoints into the attention ladder:
 * 1. Birthdays within 7 days.
 * 2. Contacts located in cities of upcoming trips in the next 14 days.
 */
const people: DecisionKind<PeopleSource, PersonDecisionRow> = {
  key: "people",
  band: 540,
  label: "People & Relationships",
  capability: "entities",
  dependsOn: ["trip"],
  view: "PeopleRow",

  load: async () => {
    const list = await entities.list("person").catch(() => []);
    return { people: list };
  },

  rows: (source, ctx) => {
    const results: PersonDecisionRow[] = [];
    const tripSource = ctx.peerSource<TripPlan[]>("trip");
    const trips = Array.isArray(tripSource) ? tripSource : [];
    const upcomingTrips = trips.filter((t) => t.date_start >= ctx.todayKey);

    const now = new Date(`${ctx.todayKey}T12:00:00Z`);
    const currentYear = now.getFullYear();

    for (const person of source.people) {
      // 1. Birthday radar (next 7 days)
      const bval = person.values?.birthday?.value;
      if (typeof bval === "string" && /^\d{4}-\d{2}-\d{2}$/.test(bval)) {
        const monthDay = bval.slice(5);
        const thisYearDate = new Date(`${currentYear}-${monthDay}T12:00:00Z`);
        let targetDate = thisYearDate;
        if (thisYearDate.getTime() < now.getTime() - 86400000) {
          targetDate = new Date(`${currentYear + 1}-${monthDay}T12:00:00Z`);
        }
        const diffDays = Math.round((targetDate.getTime() - now.getTime()) / 86400000);
        if (diffDays >= 0 && diffDays <= 7) {
          const formattedTarget = `${targetDate.getFullYear()}-${monthDay}`;
          const isToday = diffDays === 0;
          const isTomorrow = diffDays === 1;
          results.push({
            person,
            reason: "birthday",
            title: isToday
              ? `${person.name}'s birthday is today!`
              : isTomorrow
                ? `${person.name}'s birthday is tomorrow`
                : `${person.name}'s birthday in ${diffDays} days`,
            detail: `Born ${bval}. Reach out to wish them a happy birthday.`,
            daysUntil: diffDays,
            targetDate: formattedTarget,
          });
          continue;
        }
      }

      // 2. Trip proximity: does this person live in an upcoming trip destination?
      const primaryCity = person.facts.find(
        (f) => f.predicate === "home_base" || f.predicate === "away",
      )?.place;

      if (primaryCity) {
        const cityLower = primaryCity.toLowerCase();
        const matchingTrip = upcomingTrips.find((t) => {
          const inDest = t.destinations.some((d) => d.name.toLowerCase().includes(cityLower) || cityLower.includes(d.name.toLowerCase()));
          const inStages = t.stages.some((s) => s.destination.name.toLowerCase().includes(cityLower) || cityLower.includes(s.destination.name.toLowerCase()));
          return inDest || inStages;
        });

        if (matchingTrip) {
          const tripDays = ctx.daysUntil(matchingTrip.date_start);
          if (tripDays >= 0 && tripDays <= 14) {
            results.push({
              person,
              reason: "trip_proximity",
              title: `${person.name} is in ${primaryCity}`,
              detail: `You have an upcoming trip to ${matchingTrip.title || primaryCity} starting ${matchingTrip.date_start}. Plan a meetup!`,
              daysUntil: tripDays,
              targetDate: matchingTrip.date_start,
              city: primaryCity,
            });
            continue;
          }
        }
      }
    }

    return results;
  },

  id: (row) => `${row.reason}-${row.person.id}`,
  title: (row) => row.title,
  urgency: (row) => {
    if (row.reason === "birthday") {
      const days = row.daysUntil ?? 7;
      if (days === 0) return RESCALE(950, 1000);
      if (days === 1) return RESCALE(900, 1000);
      return RESCALE(Math.max(100, 800 - days * 80), 1000);
    }
    if (row.reason === "trip_proximity") {
      const days = row.daysUntil ?? 14;
      return RESCALE(Math.max(100, 750 - days * 40), 1000);
    }
    return RESCALE(300, 1000);
  },

  href: (row) => link(`/people?id=${encodeURIComponent(row.person.id)}`),

  whyHere: (row) => {
    if (row.reason === "birthday") {
      if (row.daysUntil === 0) return "Birthday today — reach out to celebrate.";
      if (row.daysUntil === 1) return "Birthday tomorrow.";
      return `Birthday in ${row.daysUntil} days.`;
    }
    if (row.reason === "trip_proximity") {
      return `Lives in ${row.city ?? "destination"} which you are visiting in ${row.daysUntil} days.`;
    }
    return row.detail;
  },

  startOrDueAt: (row) => row.targetDate,
  candidateStatus: () => "open",
  dataClass: (row) => ((row as unknown as { data_class?: unknown })?.data_class as import("../decisions").DataClass) ?? null,
  processingRoute: () => "local",
};

export default people;
