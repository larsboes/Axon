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
    reference_column: 'Reference',
    currency_column: 'Currency',
    default_currency: 'EUR',
    source_account: 'assets:bank:checking',
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
  });

  test('an unavailable profile cannot select another mapping', () => {
    expect(selectedCsvMapping(profiles, '4')).toBeNull();
    expect(selectedCsvMapping(profiles, 'not-an-index')).toBeNull();
  });
});
