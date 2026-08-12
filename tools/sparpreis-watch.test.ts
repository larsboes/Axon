import { describe, expect, test } from "bun:test";

import { dropped, previousCheapest, railWatchesOf, watchKey } from "./sparpreis-watch.ts";

describe("railWatchesOf", () => {
  const railItem = {
    item_type: "option_set",
    external_id: "split:8000207:8000105",
    payload: { query: { from: "8000207", to: "8000105", time: "2026-09-01T08:00:00", bc: 25 } },
  };

  test("a rail option_set becomes a watch with its fare context", () => {
    const watches = railWatchesOf("p1", [railItem]);
    expect(watches).toEqual([
      {
        planId: "p1",
        from: "8000207",
        to: "8000105",
        time: "2026-09-01T08:00:00",
        bc: 25,
        dTicket: false,
        firstClass: false,
      },
    ]);
  });

  test("accommodation queries and the watch's own observations are not watches", () => {
    const accommodation = {
      item_type: "option_set",
      external_id: "booking.com:berlin",
      payload: { query: { from: "coordinate-anchor", to: "52.52,13.40", check_in: "2026-10-07" } },
    };
    const ownObservation = {
      item_type: "option_set",
      external_id: "sparpreis-watch:8000207:8000105:2026-09-01T08:00:00:2026-08-11",
      payload: { query: { from: "8000207", to: "8000105", time: "2026-09-01T08:00:00" } },
    };
    const stay = { item_type: "stay", external_id: "booking.com:1", payload: {} };
    expect(railWatchesOf("p1", [accommodation, ownObservation, stay])).toEqual([]);
  });
});

describe("previousCheapest", () => {
  const key = "8000207:8000105:2026-09-01T08:00:00:bc25";
  const observation = (day: string, prices: Array<number | null>) => ({
    item_type: "option_set",
    external_id: `sparpreis-watch:${key}:${day}`,
    payload: { options: prices.map((total_price) => ({ total_price })) },
  });

  test("the newest observation's cheapest fare wins", () => {
    const items = [
      observation("2026-08-10", [29.99, 45.0]),
      observation("2026-08-11", [35.99, null, 52.0]),
    ];
    expect(previousCheapest(items, key)).toBe(35.99);
  });

  test("no prior observation means null, and a different watch key does not bleed in", () => {
    expect(previousCheapest([], key)).toBeNull();
    const other = observation("2026-08-11", [9.99]);
    other.external_id = "sparpreis-watch:8000000:8000001:2026-09-01T08:00:00:2026-08-11";
    expect(previousCheapest([other], key)).toBeNull();
  });
});

describe("dropped", () => {
  test("a real drop counts, float noise and first observations do not", () => {
    expect(dropped(35.99, 29.99)).toBe(true);
    expect(dropped(29.99, 29.985)).toBe(false);
    expect(dropped(null, 29.99)).toBe(false);
    expect(dropped(29.99, 35.99)).toBe(false);
  });
});

describe("watchKey", () => {
  test("fare context is part of the identity, absent context is absent", () => {
    expect(
      watchKey({ planId: "p", from: "1", to: "2", time: "2026-09-01T08:00:00", bc: 25 }),
    ).toBe("1:2:2026-09-01T08:00:00:bc25");
    expect(watchKey({ planId: "p", from: "1", to: "2", time: "2026-09-01T08:00:00" })).toBe(
      "1:2:2026-09-01T08:00:00",
    );
  });
});
