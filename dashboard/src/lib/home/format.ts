/** Date and count wording the ladder's rows share, so eight rows cannot phrase it eight ways. */

export function localDateKey(date: Date): string {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

/** Whole days from `from` to `value`, both read at local noon so DST cannot shift one. */
export function daysUntil(value: string, from: Date): number {
  const date = new Date(`${value.slice(0, 10)}T12:00:00`);
  if (Number.isNaN(date.getTime())) return 365;
  const start = new Date(from);
  start.setHours(12, 0, 0, 0);
  return Math.ceil((date.getTime() - start.getTime()) / 86_400_000);
}

export function dateLabel(value: string): string {
  const date = new Date(`${value.slice(0, 10)}T12:00:00`);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

export function relativeDate(value: string, now = new Date()): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const diff = Math.round((now.getTime() - date.getTime()) / 86_400_000);
  if (diff <= 0) return "today";
  if (diff === 1) return "yesterday";
  if (diff < 7) return `${diff} days ago`;
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

export function countLabel(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export function sentenceCase(value: string): string {
  return value.charAt(0).toLocaleUpperCase("en-GB") + value.slice(1);
}

/** The one place the row's meta line is assembled, so no row invents a fourth separator. */
export function metaLine(...parts: (string | null | undefined | false)[]): string {
  return parts.filter(Boolean).join(" · ");
}
