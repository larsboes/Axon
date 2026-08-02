/**
 * Deterministic pseudo-randomness for the audio engine.
 *
 * The engine used Math.random() for every stochastic decision — chord walk, note
 * choice, detune, note probability — which meant a mix that sounded right once
 * could not be heard again, and two versions of the engine could not be compared
 * on the same material. mulberry32 is 32 bits of state and one multiply per draw:
 * uniform enough for note choice, small enough that the seed is the whole session.
 */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A fresh seed. The one place Math.random() is still the right call — picking
 *  what to be deterministic about is not itself a musical decision. */
export function randomSeed(): number {
  return (Math.random() * 0xffffffff) >>> 0;
}

/** Base36 so a seed is short enough to read off the screen and type back in. */
export function formatSeed(seed: number): string {
  return (seed >>> 0).toString(36);
}

/** Inverse of formatSeed; NaN-safe, returns null on anything unparseable. */
export function parseSeed(text: string): number | null {
  const n = parseInt(text.trim(), 36);
  return Number.isFinite(n) ? n >>> 0 : null;
}
