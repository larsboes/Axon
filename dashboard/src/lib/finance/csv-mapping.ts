import type { CsvMapping, CsvMappingProfile } from '../api';

export function selectedCsvMapping(
  profiles: CsvMappingProfile[],
  selectedIndex: string,
): CsvMapping | null {
  if (selectedIndex === '') return null;
  const index = Number(selectedIndex);
  if (!Number.isInteger(index)) return null;
  const profile = profiles[index];
  return profile
    ? {
        ...profile.mapping,
        amount_sign: profile.mapping.amount_sign ?? 'as_provided',
        date_formats: [
          ...(profile.mapping.date_formats ?? [
            'iso_year_month_day',
            'day_month_year_dots',
          ]),
        ],
        row_policy: profile.mapping.row_policy ?? 'strict',
      }
    : null;
}
