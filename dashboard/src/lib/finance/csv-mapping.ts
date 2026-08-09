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
        amount_rounding: profile.mapping.amount_rounding ?? 'reject',
        default_outflow_account:
          profile.mapping.default_outflow_account ?? 'expenses:uncategorized',
        default_inflow_account:
          profile.mapping.default_inflow_account ?? 'income:uncategorized',
        categorization_rules: (profile.mapping.categorization_rules ?? []).map((rule) => ({
          ...rule,
          description_contains_any: [...rule.description_contains_any],
          description_starts_with_any: [...(rule.description_starts_with_any ?? [])],
          field_equals_any: (rule.field_equals_any ?? []).map((matcher) => ({
            ...matcher,
            values: [...matcher.values],
          })),
        })),
        row_filter: profile.mapping.row_filter
          ? {
              ...profile.mapping.row_filter,
              include_values: [...profile.mapping.row_filter.include_values],
            }
          : null,
        categorization_columns: [...(profile.mapping.categorization_columns ?? [])],
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
