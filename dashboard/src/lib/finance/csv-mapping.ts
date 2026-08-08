import type { CsvMapping, CsvMappingProfile } from '../api';

export function selectedCsvMapping(
  profiles: CsvMappingProfile[],
  selectedIndex: string,
): CsvMapping | null {
  if (selectedIndex === '') return null;
  const index = Number(selectedIndex);
  if (!Number.isInteger(index)) return null;
  const profile = profiles[index];
  return profile ? { ...profile.mapping } : null;
}
