export type PresetKey = 'edm' | 'ambient' | 'lofi' | 'focus' | 'relax' | 'sleep';

export type LayerKey = 'drums' | 'bass' | 'pads' | 'melody' | 'texture';

export type ScenarioKey = 'deep-work' | 'reading' | 'reset' | 'wind-down' | 'timer';

export interface SoundscapeSession {
  scenario: ScenarioKey;
  duration_ms: number;
  elapsed_ms: number;
  running_since_ms: number | null;
}

/**
 * How much of each layer is in the mix, 0–1. Not booleans: "pads forward, drums
 * almost out" is a mix, and mute is just the bottom of that range.
 */
export type AudioLayers = Record<LayerKey, number>;

export interface AudioParams {
  pace: number;
  density: number;
  brightness: number;
  space: number;
  pulse: number;
  texture: number;
}

export interface MusicEvent {
  t: number;
  type: 'kick' | 'note' | 'bass' | 'chord';
  midi?: number;
}

export interface FluidConfig {
  simRes: number;
  dyeRes: number;
  pressureIters: number;
  curl: number;
  velDiss: number;
  dyeDiss: number;
  splatForce: number;
  maxDpr: number;
}

export interface PresetInfo {
  label: string;
  bpm?: number;
}
