import { describe, expect, test } from "bun:test";

import type {
  CalendarCandidateVerdict,
  PlaceRef,
  ScoutingOpportunity,
  TripPlan,
} from "../src/lib/api";
import {
  assessTravelCandidates,
  calendarCandidatesFor,
  findDestinationMatch,
  haversineKm,
} from "../src/lib/travel/travel-candidates";

const place = (id: string, latitude: number | null, longitude: number | null): PlaceRef => ({
  id,
  name: id,
  kind: "city",
  latitude,
  longitude,
});

const plan = (id = "trip:plan:one"): TripPlan => ({
  id,
  title: id,
  origin: place("origin", 9, 9),
  destinations: [place("destination", 10, 10)],
  date_start: "2026-10-10",
  date_end: "2026-10-20",
  interests: "",
  status: "draft",
  travelers: [],
  transport_modes: ["train"],
  stages: [],
  cover_image_url: null,
  source: null,
  created_at: "fixture",
  updated_at: "fixture",
});

const opportunity = (id: string, latitude = 10.1, longitude = 10): ScoutingOpportunity => ({
  id,
  opportunity_type: "event",
  source: "fixture",
  title: id,
  city: "Same name on purpose",
  starts_at: "2026-10-12T18:00:00Z",
  ends_at: "2026-10-12T20:00:00Z",
  location: "Fixture venue",
  score: 0.5,
  matched_focus: "fixture",
  rationale: "fixture",
  url: `https://example.test/${id}`,
  vault_link: null,
  status: "new",
  country_code: "XX",
  latitude,
  longitude,
  event_route: {
    route: "travel_candidate",
    basis: "coordinates",
    reason: "outside the home radius",
    distance_km: 500,
  },
});

const verdict = (
  id: string,
  result: CalendarCandidateVerdict["verdict"],
): CalendarCandidateVerdict => ({
  id,
  verdict: result,
  starts_at: "2026-10-12",
  ends_at: "2026-10-13",
  already_in_calendar: false,
  evidence: [],
});

describe("travel candidate destination matching", () => {
  test("uses real distance and an inclusive trip window", () => {
    const candidate = opportunity("near");
    const match = findDestinationMatch(candidate, [plan()], "2026-08-04");
    expect(match?.destination.id).toBe("destination");
    expect(match?.distance_km).toBeLessThan(75);

    candidate.starts_at = "2026-10-20T23:00:00Z";
    expect(findDestinationMatch(candidate, [plan()], "2026-08-04")).not.toBeNull();
    candidate.starts_at = "2026-10-21T00:00:00Z";
    expect(findDestinationMatch(candidate, [plan()], "2026-08-04")).toBeNull();
  });

  test("includes the destination-radius boundary and rejects the first point outside it", () => {
    expect(
      findDestinationMatch(opportunity("inside", 10.67, 10), [plan()], "2026-08-04"),
    ).not.toBeNull();
    expect(
      findDestinationMatch(opportunity("outside", 10.68, 10), [plan()], "2026-08-04"),
    ).toBeNull();
  });

  test("same city text cannot rescue a far coordinate", () => {
    const candidate = opportunity("far", 40, 40);
    expect(candidate.city).toBe("Same name on purpose");
    expect(findDestinationMatch(candidate, [plan()], "2026-08-04")).toBeNull();
  });

  test("missing and half coordinates never become zero zero", () => {
    const missing = opportunity("missing");
    missing.latitude = null;
    missing.longitude = null;
    expect(findDestinationMatch(missing, [plan()], "2026-08-04")).toBeNull();

    const half = opportunity("half");
    half.latitude = null;
    half.longitude = 0;
    expect(findDestinationMatch(half, [plan()], "2026-08-04")).toBeNull();
    expect(haversineKm({ latitude: 0, longitude: 0 }, { latitude: 0, longitude: 0 })).toBe(0);
  });

  test("chooses the nearest match stably", () => {
    const later = plan("trip:plan:later");
    later.destinations = [place("nearer", 10.05, 10)];
    const earlier = plan("trip:plan:earlier");
    earlier.destinations = [place("further", 10.2, 10)];
    expect(
      findDestinationMatch(opportunity("candidate"), [earlier, later], "2026-08-04")?.plan.id,
    ).toBe("trip:plan:later");
  });
});

describe("travel candidate calendar routing", () => {
  test("only unmatched dated candidates enter the Calendar batch", () => {
    const matched = opportunity("matched");
    const unmatched = opportunity("unmatched", 40, 40);
    const undated = opportunity("undated", 40, 40);
    undated.starts_at = "";
    const dismissed = opportunity("dismissed", 40, 40);
    dismissed.status = "dismissed";
    const past = opportunity("past", 40, 40);
    past.starts_at = "2026-07-01T18:00:00Z";
    past.ends_at = "2026-07-01T20:00:00Z";

    expect(
      calendarCandidatesFor(
        [matched, unmatched, undated, dismissed, past],
        [plan()],
        "2026-08-04",
      ),
    ).toEqual([
      {
        id: "unmatched",
        starts_at: unmatched.starts_at,
        ends_at: unmatched.ends_at,
      },
    ]);
  });

  test("maps soft and hard verdicts and keeps hard conflicts last", () => {
    const free = opportunity("free", 40, 40);
    const movable = opportunity("movable", 41, 41);
    const blocked = opportunity("blocked", 42, 42);
    const verdicts = new Map([
      [free.id, verdict(free.id, "free")],
      [movable.id, verdict(movable.id, "needs-travel-day")],
      [blocked.id, verdict(blocked.id, "conflicts")],
    ]);

    const assessed = assessTravelCandidates(
      [blocked, movable, free],
      [],
      verdicts,
      "2026-08-04",
      true,
    );
    expect(assessed.map((candidate) => candidate.state)).toEqual([
      "free_window",
      "needs_travel_day",
      "conflicts",
    ]);
    expect(assessed.at(-1)?.reason).toContain("do not plan");
  });

  test("service, date, and location gaps remain explicit", () => {
    const sparse = opportunity("sparse");
    sparse.latitude = null;
    sparse.longitude = null;
    const undated = opportunity("undated");
    undated.starts_at = "";
    const assessed = assessTravelCandidates(
      [sparse, undated],
      [],
      new Map(),
      "2026-08-04",
      false,
    );
    expect(assessed[0].state).toBe("calendar_unavailable");
    expect(assessed[0].reason).toContain("no complete coordinate");
    expect(assessed[1].state).toBe("date_unresolved");
  });
});
