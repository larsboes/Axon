import { describe, expect, test } from 'bun:test';
import type { CsvMappingProfile } from '../src/lib/api';
import { selectedCsvMapping } from '../src/lib/finance/csv-mapping';

const profiles: CsvMappingProfile[] = [{
  label: 'Synthetic semicolon export',
  mapping: {
    delimiter: ';',
    decimal_separator: ',',
    date_column: 'Date',
    amount_column: 'Amount',
    description_column: 'Description',
    categorization_columns: ['Statement label'],
    reference_column: 'Reference',
    currency_column: 'Currency',
    default_currency: 'EUR',
    source_account: 'assets:bank:checking',
    default_outflow_account: 'expenses:uncategorized',
    default_inflow_account: 'income:uncategorized',
    categorization_rules: [{
      description_contains_any: ['synthetic service'],
      description_starts_with_any: [],
      field_equals_any: [{ column: 'Entry type', values: ['PURCHASE'] }],
      direction: 'outflow',
      account: 'expenses:software',
      confidence_basis_points: 9500,
    }],
    row_filter: null,
    amount_sign: 'as_provided',
    amount_rounding: 'reject',
    date_formats: ['iso_year_month_day', 'day_month_year_dots'],
    row_policy: 'strict',
  },
}];

describe('selectedCsvMapping', () => {
  test('manual entry does not replace the current mapping', () => {
    expect(selectedCsvMapping(profiles, '')).toBeNull();
  });

  test('a private profile is copied before the UI edits it', () => {
    const selected = selectedCsvMapping(profiles, '0');
    expect(selected).toEqual(profiles[0].mapping);
    expect(selected).not.toBe(profiles[0].mapping);
    expect(selected?.date_formats).not.toBe(profiles[0].mapping.date_formats);
    expect(selected?.categorization_columns).not.toBe(profiles[0].mapping.categorization_columns);
    expect(selected?.categorization_rules).not.toBe(profiles[0].mapping.categorization_rules);
    expect(selected?.categorization_rules[0].description_contains_any)
      .not.toBe(profiles[0].mapping.categorization_rules[0].description_contains_any);
    expect(selected?.categorization_rules[0].description_starts_with_any)
      .not.toBe(profiles[0].mapping.categorization_rules[0].description_starts_with_any);
    expect(selected?.categorization_rules[0].field_equals_any?.[0].values)
      .not.toBe(profiles[0].mapping.categorization_rules[0].field_equals_any?.[0].values);
  });

  test('an unavailable profile cannot select another mapping', () => {
    expect(selectedCsvMapping(profiles, '4')).toBeNull();
    expect(selectedCsvMapping(profiles, 'not-an-index')).toBeNull();
  });
});
