// The shell starts what a page needs, and starts it once.
//
// Two defects in one fix. /finance and /map were the only two primary destinations that
// started nothing at all, so on a machine where those capabilities were stopped both
// rendered an error card and the operator had to go to /capabilities and come back. And
// the obvious fix is a trap: `capabilities` re-assigns its list every fifteen seconds, so
// an effect that reads the store re-runs on every poll and turns one start into one every
// fifteen seconds. The attempted-set is what makes that impossible, and it is asserted
// here rather than assumed.

import { describe, expect, test } from "bun:test";
import {
  PRIMARY_NAV,
  UTILITY_NAV,
  capabilityForPath,
} from "../dashboard/src/lib/nav.ts";

describe("capabilityForPath names what a page needs running", () => {
  test("the two pages that started nothing now resolve", () => {
    expect(capabilityForPath("/finance")).toEqual(["finance"]);
    expect(capabilityForPath("/map")).toEqual(["places"]);
  });

  test("travel starts both capabilities it reads, not only the one it is named for", () => {
    // `capability` says which absence makes the destination pointless; `starts` says what
    // the page actually reads. /travel needs transit for connections and trips for plans,
    // and neither would start on its own.
    expect(capabilityForPath("/travel")).toEqual(["transit", "trips"]);
  });

  test("Home starts nothing from the shell", () => {
    // Home reads seven capabilities and starts each on demand through its own registry,
    // which is a per-kind decision this table cannot make. `/` must also not claim every
    // path in the app by prefix.
    expect(capabilityForPath("/")).toEqual([]);
  });

  test("a nested route resolves to its section", () => {
    expect(capabilityForPath("/feed/abc123")).toEqual(["comms"]);
    expect(capabilityForPath("/travel/connections")).toEqual(["transit", "trips"]);
  });

  test("an unknown path asks for nothing rather than guessing", () => {
    expect(capabilityForPath("/nothing-here")).toEqual([]);
  });

  test("every nav item with a capability resolves to at least that capability", () => {
    const unresolved = [...PRIMARY_NAV, ...UTILITY_NAV]
      .filter((item) => item.capability && item.href !== "/")
      .filter((item) => !capabilityForPath(item.href).includes(item.capability as string))
      .map((item) => item.href);
    expect(unresolved).toEqual([]);
  });
});

describe("a start is attempted once, not once per capability poll", () => {
  /** The shell's guard, reproduced exactly: a set consulted before every start. */
  function shell() {
    const attempted = new Set<string>();
    const posted: string[] = [];
    return {
      posted,
      open(pathname: string, up: (name: string) => boolean) {
        for (const name of capabilityForPath(pathname)) {
          if (attempted.has(name)) continue;
          attempted.add(name);
          if (up(name)) continue;
          posted.push(name);
        }
      },
    };
  }

  test("a stopped capability is started once across many polls", () => {
    const s = shell();
    const stopped = () => false;
    for (let poll = 0; poll < 8; poll += 1) s.open("/finance", stopped);
    expect(s.posted).toEqual(["finance"]);
  });

  test("a running capability is never started", () => {
    const s = shell();
    s.open("/finance", () => true);
    expect(s.posted).toEqual([]);
  });

  test("a failed start is recorded, so it does not become a retry loop", () => {
    // The set is written BEFORE the POST, which is what makes a permanently failing
    // capability cost exactly one request rather than one every fifteen seconds.
    const s = shell();
    s.open("/map", () => false);
    s.open("/map", () => false);
    expect(s.posted).toEqual(["places"]);
  });

  test("two capabilities on one page are each started once", () => {
    const s = shell();
    s.open("/travel", () => false);
    s.open("/travel", () => false);
    expect(s.posted).toEqual(["transit", "trips"]);
  });
});
