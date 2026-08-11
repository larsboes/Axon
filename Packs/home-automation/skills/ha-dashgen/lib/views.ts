/**
 * View and dashboard builders.
 *
 * Sections, not masonry. Masonry packs cards into whichever column is shortest, which is why
 * the dashboard this replaces had ragged columns and dead space below the fold on every view.
 * A sections view places cards on a grid the author controls and reflows predictably between
 * a 390px phone and a landscape tablet, which is the whole reason two layouts can share one
 * content model.
 */

import type { Card } from "./cards.ts";

export type Section = {
  type: "grid";
  column_span?: number;
  cards: Card[];
};

export type View = Record<string, unknown>;

export const section = (cards: Card[], columnSpan?: number): Section => ({
  type: "grid",
  ...(columnSpan ? { column_span: columnSpan } : {}),
  cards,
});

/** A section that only renders when a condition holds. Silence when the house is fine. */
export const conditionalSection = (
  conditions: Record<string, unknown>[],
  cards: Card[],
  columnSpan?: number,
): Section =>
  ({
    type: "grid",
    ...(columnSpan ? { column_span: columnSpan } : {}),
    cards,
    visibility: conditions,
  }) as Section;

export const view = (opts: {
  title: string;
  path: string;
  icon: string;
  sections: Section[];
  badges?: Card[];
  maxColumns?: number;
  denseSection?: boolean;
  subview?: boolean;
}): View => ({
  title: opts.title,
  path: opts.path,
  icon: opts.icon,
  type: "sections",
  max_columns: opts.maxColumns ?? 4,
  dense_section_placement: opts.denseSection ?? true,
  ...(opts.subview ? { subview: true } : {}),
  ...(opts.badges?.length ? { badges: opts.badges } : {}),
  sections: opts.sections,
});

export const dashboard = (title: string, views: View[]): Record<string, unknown> => ({
  title,
  views,
});

/* ── visibility conditions ──────────────────────────────────────────────────── */

export const whenState = (entity: string, state: string | string[]): Record<string, unknown> => ({
  condition: "state",
  entity,
  ...(Array.isArray(state) ? { state_not: undefined, state } : { state }),
});

export const whenNumeric = (
  entity: string,
  opts: { above?: number; below?: number },
): Record<string, unknown> => ({
  condition: "numeric_state",
  entity,
  ...(opts.above !== undefined ? { above: opts.above } : {}),
  ...(opts.below !== undefined ? { below: opts.below } : {}),
});

/** Screen-width gate. The one place a single model legitimately renders differently. */
export const whenScreen = (media: string): Record<string, unknown> => ({
  condition: "screen",
  media_query: media,
});
