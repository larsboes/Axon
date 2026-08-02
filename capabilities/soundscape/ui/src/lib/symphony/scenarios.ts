import type { PresetKey, ScenarioKey } from './types';

export interface ScenarioInfo {
  key: Exclude<ScenarioKey, 'timer'>;
  label: string;
  detail: string;
  minutes: number;
  preset: PresetKey;
  energy: number;
}

export const QUICK_SCENARIOS: ScenarioInfo[] = [
  {
    key: 'deep-work',
    label: 'Deep focus',
    detail: 'A clear pulse with little distraction',
    minutes: 50,
    preset: 'focus',
    energy: 0.72,
  },
  {
    key: 'reading',
    label: 'Reading',
    detail: 'Calm focus without a strong beat',
    minutes: 25,
    preset: 'focus',
    energy: 0.42,
  },
  {
    key: 'reset',
    label: 'Short reset',
    detail: 'Step back for ten minutes',
    minutes: 10,
    preset: 'relax',
    energy: 0.38,
  },
  {
    key: 'wind-down',
    label: 'Wind down',
    detail: 'Ease into a quiet evening',
    minutes: 45,
    preset: 'sleep',
    energy: 0.24,
  },
];

export function scenarioLabel(key: ScenarioKey): string {
  if (key === 'timer') return 'Timer';
  return QUICK_SCENARIOS.find((scenario) => scenario.key === key)?.label ?? key;
}
