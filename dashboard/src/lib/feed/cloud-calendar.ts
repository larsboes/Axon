import type {
  CalendarNewEntry,
  CloudContentAnalysis,
  ContentSource,
  DataClass,
} from "$lib/api";

export interface CloudCalendarCandidate {
  key: string;
  entry: CalendarNewEntry;
}

interface CandidateInput {
  field: "important_dates" | "action_items";
  index: number;
  date: string | null;
  title: string;
  evidence: string | null;
  kind: "event" | "deadline";
}

function nextDay(value: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(Date.UTC(year, month - 1, day));
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) return null;
  date.setUTCDate(date.getUTCDate() + 1);
  return date.toISOString().slice(0, 10);
}

function externalId(source: ContentSource, itemId: string, candidate: CandidateInput): string {
  const title = candidate.title.trim().toLocaleLowerCase("en").replace(/\s+/g, " ").slice(0, 160);
  return `content-analysis:${source}:${itemId}:${candidate.field}:${candidate.date}:${title}`;
}

export function cloudCalendarCandidates(input: {
  source: ContentSource;
  itemId: string;
  jobId: string;
  dataClass: DataClass;
  result: CloudContentAnalysis;
}): CloudCalendarCandidate[] {
  const sourceCandidates: CandidateInput[] = [
    ...input.result.important_dates.map((date, index) => ({
      field: "important_dates" as const,
      index,
      date: date.date,
      title: date.label,
      evidence: date.source_text,
      kind: "event" as const,
    })),
    ...input.result.action_items.map((action, index) => ({
      field: "action_items" as const,
      index,
      date: action.due_date,
      title: action.text,
      evidence: null,
      kind: "deadline" as const,
    })),
  ];

  const seen = new Set<string>();
  return sourceCandidates.flatMap((candidate) => {
    const endsAt = candidate.date ? nextDay(candidate.date) : null;
    const title = candidate.title.trim();
    if (!candidate.date || !endsAt || !title) return [];
    const id = externalId(input.source, input.itemId, candidate);
    if (seen.has(id)) return [];
    seen.add(id);
    return [{
      key: `${candidate.field}:${candidate.index}`,
      entry: {
        kind: candidate.kind,
        commitment: "possible",
        title,
        starts_at: candidate.date,
        ends_at: endsAt,
        all_day: true,
        source: "comms",
        external_id: id,
        notes: candidate.evidence,
        payload: {
          schema_version: "calendar-proposal-provenance-v1",
          origin: {
            capability: "comms",
            source: input.source,
            item_id: input.itemId,
            job_id: input.jobId,
            field: candidate.field,
            index: candidate.index,
          },
          data_class: input.dataClass,
          analysis_schema_version: input.result.schema_version,
          importance: input.result.importance,
          importance_rationale: input.result.importance_rationale,
          evidence: candidate.evidence,
        },
      },
    }];
  });
}
