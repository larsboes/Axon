import { describe, expect, test } from "bun:test";
import { cloudCalendarCandidates } from "../src/lib/feed/cloud-calendar";

const result = {
  schema_version: "cloud-content-analysis-v1" as const,
  summary: "A trip and a follow-up need review.",
  importance: "high" as const,
  importance_rationale: "The document contains a fixed date.",
  important_dates: [
    { label: "Theme park visit", date: "2026-08-10", source_text: "on 10 August" },
    { label: "Unresolved mention", date: null, source_text: "next summer" },
  ],
  action_items: [
    { text: "Confirm attendance", due_date: "2026-08-08" },
    { text: "Ask a question", due_date: null },
  ],
  topics: ["travel"],
};

describe("cloud analysis Calendar proposals", () => {
  test("maps only resolved dates into review-only external entries", () => {
    const candidates = cloudCalendarCandidates({
      source: "mail",
      itemId: "thread-1",
      jobId: "job-1",
      dataClass: "personal",
      result,
    });

    expect(candidates).toHaveLength(2);
    expect(candidates[0].entry).toMatchObject({
      kind: "event",
      commitment: "possible",
      starts_at: "2026-08-10",
      ends_at: "2026-08-11",
      source: "comms",
    });
    expect(candidates[1].entry.kind).toBe("deadline");
    expect(candidates[1].entry.starts_at).toBe("2026-08-08");
  });

  test("uses stable content identity rather than the provider job id", () => {
    const first = cloudCalendarCandidates({ source: "mail", itemId: "thread-1", jobId: "job-1", dataClass: "personal", result });
    const rerun = cloudCalendarCandidates({ source: "mail", itemId: "thread-1", jobId: "job-2", dataClass: "personal", result });
    expect(first.map((candidate) => candidate.entry.external_id)).toEqual(
      rerun.map((candidate) => candidate.entry.external_id),
    );
  });

  test("rejects impossible and non-ISO dates", () => {
    const invalid = {
      ...result,
      important_dates: [
        { label: "Impossible", date: "2026-02-30", source_text: "30 February" },
        { label: "Vague", date: "August 10", source_text: "August 10" },
      ],
      action_items: [],
    };
    expect(cloudCalendarCandidates({ source: "feed", itemId: "item-1", jobId: "job-1", dataClass: "public", result: invalid })).toEqual([]);
  });
});
