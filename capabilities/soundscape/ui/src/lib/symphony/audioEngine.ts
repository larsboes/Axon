import type { PresetKey, AudioLayers, AudioParams, MusicEvent, PresetInfo } from './types';
import { mulberry32, randomSeed } from './rng';

export const PALETTES: Record<PresetKey, Array<[number, number, number]>> = {
  edm:     [[0.55, 0.18, 1.0], [0.95, 0.2, 0.85], [0.3, 0.08, 0.95], [1.0, 0.25, 0.55]],
  ambient: [[0.08, 0.55, 0.95], [0.15, 0.9, 0.75], [0.95, 0.65, 0.25], [0.35, 0.45, 1.0]],
  lofi:    [[1.0, 0.55, 0.25], [0.95, 0.35, 0.4], [0.75, 0.45, 0.85], [1.0, 0.78, 0.35]],
  focus:   [[0.1, 0.5, 0.95], [0.2, 0.85, 0.9], [0.35, 0.4, 1.0], [0.15, 0.9, 0.65]],
  relax:   [[1.0, 0.6, 0.3], [0.9, 0.42, 0.5], [0.3, 0.7, 0.7], [1.0, 0.8, 0.45]],
  sleep:   [[0.28, 0.22, 0.75], [0.42, 0.26, 0.85], [0.16, 0.16, 0.55], [0.5, 0.32, 0.95]],
};

export const SCAPES = {
  focus: {
    bpmBase: 96, root: 48, scale: [0, 2, 4, 7, 9], fluid: 1.0,
    defaults: { pace: 0.55, density: 0.5, brightness: 0.6, space: 0.35, pulse: 0.55, texture: 0.35 }
  },
  relax: {
    bpmBase: 66, root: 45, scale: [0, 3, 5, 7, 10], fluid: 0.7,
    defaults: { pace: 0.4, density: 0.35, brightness: 0.42, space: 0.6, pulse: 0.3, texture: 0.45 }
  },
  sleep: {
    bpmBase: 52, root: 38, scale: [0, 3, 7, 10], fluid: 0.45,
    defaults: { pace: 0.22, density: 0.15, brightness: 0.2, space: 0.85, pulse: 0.15, texture: 0.55 }
  }
};

/**
 * What one point on the arrangement arc does to the mix. `energy` 1 is the full
 * arrangement, 0 the emptiest point of the rest. The drum gate is deliberately
 * hard rather than faded — a layer that thins out is not felt returning.
 */
const shapeArc = (energy: number) => ({
  energy,
  density: 0.4 + 0.75 * energy,
  pulse: 0.25 + 0.75 * energy,
  brightnessAdd: -0.12 + 0.2 * energy,
  drums: energy < 0.25 ? 0 : 1,
});

const DRIFT: Record<keyof AudioParams, [number, number]> = {
  pace: [0.0035, 0],
  density: [0.0061, 2.1],
  brightness: [0.0047, 4.2],
  space: [0.0028, 1.3],
  pulse: [0.0072, 3.4],
  texture: [0.0041, 5.5],
};

export const PRESETS: Record<PresetKey, PresetInfo> = {
  edm: { label: 'CLASSICAL × EDM · 126 BPM', bpm: 126 },
  ambient: { label: 'EPIC AMBIENT · 72 BPM', bpm: 72 },
  lofi: { label: 'LO-FI · 78 BPM', bpm: 78 },
  focus: { label: 'FOCUS' },
  relax: { label: 'RELAX' },
  sleep: { label: 'SLEEP' },
};

const N = (m: number) => 440 * Math.pow(2, (m - 69) / 12);

/**
 * Everything one preset is currently sounding, on four buses. A preset change
 * builds a fresh scene and fades the outgoing one out across a bar instead of
 * abandoning its held voices mid-note, which is what made a switch a hard cut.
 */
interface SceneBus {
  dry: GainNode;
  duck: GainNode;
  rev: GainNode;
  del: GainNode;
}

export class AudioEngine {
  public ctx: AudioContext | null = null;
  public master: GainNode | null = null;
  public duck: GainNode | null = null;
  public analyser: AnalyserNode | null = null;
  public comp: DynamicsCompressorNode | null = null;
  public reverbSend: GainNode | null = null;
  public delaySend: GainNode | null = null;
  public delayNode: DelayNode | null = null;
  public noiseBuf: AudioBuffer | null = null;
  private vinylGain: GainNode | null = null;
  private vinylSource: AudioBufferSourceNode | null = null;
  private texGain: GainNode | null = null;
  private texLP: BiquadFilterNode | null = null;
  private texSource: AudioBufferSourceNode | null = null;

  /** Two convolvers so the room can change size without a click: build the new
   *  impulse on the idle one, crossfade, and let the old tail ring out. */
  private rooms: Array<{ conv: ConvolverNode; wet: GainNode }> = [];
  private roomActive = 0;
  private roomSpace = -1;
  private roomBuiltAt = -Infinity;
  private noiseRnd: () => number = mulberry32(0);

  public playing = false;
  public preset: PresetKey = 'edm';
  public energy = 0.7;
  public volume = 0.65;
  public step = 0;
  public nextTime = 0;
  private timer: number | null = null;

  public events: MusicEvent[] = [];
  private eventHeadIndex = 0;
  public freqData: Uint8Array<ArrayBuffer> | null = null;

  public layers: AudioLayers = { drums: 1, bass: 1, pads: 1, melody: 1, texture: 1 };
  public params: AudioParams = { pace: 0.5, density: 0.5, brightness: 0.5, space: 0.5, pulse: 0.4, texture: 0.4 };
  public adapt = false;
  public activityLevel = 0;

  public seed = randomSeed();
  private rnd: () => number = mulberry32(this.seed);

  private scene: SceneBus | null = null;
  /** A preset waiting for the next bar boundary. Readable so the UI can show it. */
  public pendingPreset: PresetKey | null = null;
  /** Step the current preset started on, so each one hears its own grid from 0
   *  while the global step counter keeps running underneath the transition. */
  private stepBase = 0;
  /** Context time the current preset began, which is where its arc starts. */
  private presetStartedAt = 0;

  private scapeDeg = 0;
  private scapeChord: number[] | null = null;
  /** Set by a preset change: the next chord change keeps scapeDeg instead of walking. */
  private holdDeg = false;

  private EDM_CHORDS = [[48, 51, 55], [44, 48, 51], [46, 50, 53], [43, 47, 50]];
  private EDM_ROOTS = [36, 32, 34, 31];
  private EDM_MOTIF: Record<number, [number, number]> = {
    2: [67, 6], 4: [67, 2], 6: [67, 2], 8: [63, 8],
    18: [65, 6], 20: [65, 2], 22: [65, 2], 24: [62, 8]
  };

  private AMB_CHORDS = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]];
  private AMB_ROOTS = [45, 41, 36, 43];

  private LOFI_CHORDS = [[53, 57, 60, 64], [52, 55, 59, 62], [50, 53, 57, 60], [48, 52, 55, 59]];
  private LOFI_ROOTS = [41, 40, 38, 36];
  private LOFI_PENTA = [72, 74, 76, 79, 81];

  public isScape(): boolean {
    return !!SCAPES[this.preset as keyof typeof SCAPES];
  }

  /** Restart the musical random stream. Same seed plus same params replays the
   *  same material; the noise beds keep whatever they were built with at init. */
  public setSeed(seed: number) {
    this.seed = seed >>> 0;
    this.rnd = mulberry32(this.seed);
  }

  public reseed(): number {
    this.setSeed(randomSeed());
    return this.seed;
  }

  /** Place a voice in the stereo field. Returns the node to connect onward — the
   *  gain itself when centred, so a centred voice pays for no extra node. */
  private place(g: GainNode, pan: number): AudioNode {
    if (!this.ctx || pan === 0) return g;
    const p = this.ctx.createStereoPanner();
    p.pan.value = Math.max(-1, Math.min(1, pan));
    g.connect(p);
    return p;
  }

  /**
   * How large the room should be right now, 0–1. A scape follows its `space`
   * param; a fixed performance gets the room its arrangement was written for —
   * an EDM mix wants a small one, epic ambient wants a hall.
   */
  private roomTarget(): number {
    if (this.isScape()) return this.eff('space');
    return { edm: 0.18, ambient: 0.85, lofi: 0.3 }[this.preset as 'edm' | 'ambient' | 'lofi'] ?? 0.4;
  }

  /** Decay 1.2s to 6s. The exponent falls as the room grows, so a large room does
   *  not merely last longer, it also trails away more slowly. */
  private buildImpulse(space: number): AudioBuffer | null {
    if (!this.ctx) return null;
    const rate = this.ctx.sampleRate;
    const len = Math.floor((1.2 + 4.8 * space) * rate);
    const exp = 3.2 - 1.6 * space;
    const imp = this.ctx.createBuffer(2, len, rate);
    for (let ch = 0; ch < 2; ch++) {
      const d = imp.getChannelData(ch);
      for (let i = 0; i < len; i++) {
        d[i] = (this.noiseRnd() * 2 - 1) * Math.pow(1 - i / len, exp);
      }
    }
    return imp;
  }

  /**
   * Move to a different room size. Skipped unless the change is worth the cost —
   * an impulse is up to six seconds of noise to generate, and `space` drifts
   * continuously, so rebuilding on every call would rebuild for nothing.
   */
  private setRoom(space: number, at: number) {
    if (!this.ctx || this.rooms.length < 2) return;
    if (Math.abs(space - this.roomSpace) < 0.08) return;
    if (at - this.roomBuiltAt < 4) return;

    const next = (this.roomActive + 1) % 2;
    const imp = this.buildImpulse(space);
    if (!imp) return;
    this.rooms[next].conv.buffer = imp;

    // Long enough to be inaudible as a change; the outgoing convolver keeps
    // ringing its own tail while it fades, so nothing is cut off.
    this.rooms[next].wet.gain.setTargetAtTime(1, at, 0.6);
    this.rooms[this.roomActive].wet.gain.setTargetAtTime(0, at, 0.6);
    this.roomActive = next;
    this.roomSpace = space;
    this.roomBuiltAt = at;
  }

  private newScene(): SceneBus | null {
    if (!this.ctx || !this.master || !this.duck || !this.reverbSend || !this.delaySend) return null;
    const c = this.ctx;
    const bus = (dest: AudioNode) => {
      const g = c.createGain();
      g.gain.value = 1;
      g.connect(dest);
      return g;
    };
    return {
      dry: bus(this.master),
      duck: bus(this.duck),
      rev: bus(this.reverbSend),
      del: bus(this.delaySend),
    };
  }

  /** Fade a scene out and drop it. The tail runs past the fade because a pad
   *  released into a six-second reverb is still audible after its bus hits zero. */
  private retireScene(s: SceneBus, at: number, over: number) {
    const buses = [s.dry, s.duck, s.rev, s.del];
    for (const g of buses) {
      g.gain.cancelScheduledValues(at);
      g.gain.setValueAtTime(g.gain.value, at);
      g.gain.linearRampToValueAtTime(0.0001, at + over);
    }
    const tail = Math.max(0, at + over + 4 - (this.ctx ? this.ctx.currentTime : 0));
    window.setTimeout(() => {
      for (const g of buses) g.disconnect();
    }, tail * 1000);
  }

  /**
   * The slow shape over a scape. Without it a scape is structurally identical at
   * minute 3 and minute 47 — the only thing moving is six drift LFOs at ±0.08.
   * Reads `params.pace` raw, never eff('pace'), because eff consults this.
   */
  private arc() {
    const t = this.ctx ? this.ctx.currentTime : 0;
    const elapsed = Math.max(0, t - this.presetStartedAt);

    // sleep does not come back around. It is supposed to end somewhere.
    if (this.preset === 'sleep') {
      const e = Math.max(0, 1 - elapsed / 2700);
      return { ...shapeArc(e), phase: e > 0.66 ? 'settle' : e > 0.33 ? 'drift' : 'still' };
    }

    const cycle = 1080 - 600 * this.params.pace;
    const pos = (elapsed % cycle) / cycle;
    // Starts at the top: pressing play should sound like the whole piece, and the
    // sparse part is something the arc arrives at rather than opens with.
    const e = 0.5 + 0.5 * Math.cos(2 * Math.PI * pos);
    const phase = pos < 0.25 ? 'bloom' : pos < 0.5 ? 'ebb' : pos < 0.7 ? 'rest' : 'swell';
    return { ...shapeArc(e), phase };
  }

  /** Where the arrangement is right now; null for the fixed performances, which
   *  carry their own arrangement in their step functions. */
  public get arcPhase(): string | null {
    return this.isScape() ? this.arc().phase : null;
  }

  /** Layer levels as actually played: what the operator set, scaled by the arc. */
  private mix(): AudioLayers {
    if (!this.isScape()) return this.layers;
    const a = this.arc();
    return { ...this.layers, drums: this.layers.drums * a.drums };
  }

  public eff(name: keyof AudioParams): number {
    const d = DRIFT[name];
    const t = this.ctx ? this.ctx.currentTime : 0;
    let v = this.params[name] + 0.08 * Math.sin(2 * Math.PI * d[0] * t + d[1]);
    if (this.adapt && (name === 'density' || name === 'pace' || name === 'pulse')) {
      v += (this.activityLevel - 0.3) * 0.18;
    }
    if (this.isScape()) {
      // On top of the drift, never instead of it.
      const a = this.arc();
      if (name === 'density') v *= a.density;
      else if (name === 'pulse') v *= a.pulse;
      else if (name === 'brightness') v += a.brightnessAdd;
    }
    return Math.min(1, Math.max(0, v));
  }

  public init() {
    if (this.ctx) return;
    const AudioCtx = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    const ctx = (this.ctx = new AudioCtx());

    this.master = ctx.createGain();
    this.master.gain.value = this.volume * this.volume;

    this.comp = ctx.createDynamicsCompressor();
    this.comp.threshold.value = -14;
    this.comp.ratio.value = 4;

    this.analyser = ctx.createAnalyser();
    this.analyser.fftSize = 256;
    this.freqData = new Uint8Array(this.analyser.frequencyBinCount);

    this.master.connect(this.comp);
    this.comp.connect(this.analyser);
    this.analyser.connect(ctx.destination);

    this.duck = ctx.createGain();
    this.duck.connect(this.master);

    // The beds draw from their own stream, not the musical one: they consume
    // hundreds of thousands of samples, and the note sequence must not depend on
    // this device's sample rate.
    const noiseRnd = (this.noiseRnd = mulberry32(this.seed ^ 0x9e3779b9));
    const rate = ctx.sampleRate;

    this.reverbSend = ctx.createGain();
    this.reverbSend.gain.value = 0.25;
    for (let i = 0; i < 2; i++) {
      const conv = ctx.createConvolver();
      const wet = ctx.createGain();
      wet.gain.value = i === 0 ? 1 : 0;
      this.reverbSend.connect(conv);
      conv.connect(wet);
      wet.connect(this.master);
      this.rooms.push({ conv, wet });
    }
    this.rooms[0].conv.buffer = this.buildImpulse(this.roomTarget());
    this.roomSpace = this.roomTarget();

    // Delay node
    const delay = ctx.createDelay(2.0);
    this.delayNode = delay;
    const fb = ctx.createGain();
    fb.gain.value = 0.32;
    const dwet = ctx.createGain();
    dwet.gain.value = 0.5;
    this.delaySend = ctx.createGain();
    this.delaySend.gain.value = 1;
    this.delaySend.connect(delay);
    delay.connect(fb);
    fb.connect(delay);
    delay.connect(dwet);
    dwet.connect(this.master);

    // Noise buffer
    const nb = ctx.createBuffer(1, rate, rate);
    const nd = nb.getChannelData(0);
    for (let i = 0; i < rate; i++) nd[i] = noiseRnd() * 2 - 1;
    this.noiseBuf = nb;

    // Vinyl bed
    const vin = (this.vinylSource = ctx.createBufferSource());
    vin.buffer = nb;
    vin.loop = true;
    const vlp = ctx.createBiquadFilter();
    vlp.type = 'lowpass';
    vlp.frequency.value = 4200;
    const vhp = ctx.createBiquadFilter();
    vhp.type = 'highpass';
    vhp.frequency.value = 600;
    this.vinylGain = ctx.createGain();
    this.vinylGain.gain.value = 0;
    vin.connect(vlp);
    vlp.connect(vhp);
    vhp.connect(this.vinylGain);
    this.vinylGain.connect(this.master);
    vin.start();

    // Scene texture bed
    const tex = (this.texSource = ctx.createBufferSource());
    tex.buffer = nb;
    tex.loop = true;
    this.texLP = ctx.createBiquadFilter();
    this.texLP.type = 'lowpass';
    this.texLP.frequency.value = 900;
    this.texGain = ctx.createGain();
    this.texGain.gain.value = 0;
    tex.connect(this.texLP);
    this.texLP.connect(this.texGain);
    this.texGain.connect(this.master);
    this.texGain.connect(this.reverbSend);
    tex.start();

    this.scene = this.newScene();
  }

  get bpm(): number {
    const p = PRESETS[this.preset];
    if (p.bpm) return p.bpm;
    const sc = SCAPES[this.preset as keyof typeof SCAPES];
    return sc ? sc.bpmBase * (0.75 + 0.5 * this.eff('pace')) : 120;
  }

  get spb(): number {
    return 60 / this.bpm;
  }

  public start() {
    if (!this.ctx) this.init();
    if (!this.ctx) return;
    this.ctx.resume();
    this.setVolume(this.volume);
    if (this.playing) return;
    this.playing = true;
    this.step = 0;
    this.stepBase = 0;
    this.presetStartedAt = this.ctx.currentTime;
    this.nextTime = this.ctx.currentTime + 0.1;
    this.applyPresetFx();
    this.timer = window.setInterval(() => this.schedule(), 25);
  }

  public stop() {
    this.playing = false;
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
    if (this.ctx) this.ctx.suspend();
  }

  /**
   * Queue a preset change for the next bar; returns true if it landed now and
   * false if it is pending. Selecting the pending preset a second time switches
   * immediately — that is the deliberate escape hatch from waiting for the bar.
   * Selecting the one already playing cancels a pending change.
   */
  public setPreset(p: PresetKey, opts: { immediate?: boolean; keepParams?: boolean } = {}): boolean {
    if (!this.ctx || !this.playing) {
      this.step = 0;
      this.events.length = 0;
      this.eventHeadIndex = 0;
      this.applyPreset(p, opts.keepParams === true, 0);
      return true;
    }
    if (p === this.preset) {
      this.pendingPreset = null;
      return true;
    }
    if (opts.immediate === true || this.pendingPreset === p) {
      this.applyPreset(p, opts.keepParams === true, this.ctx.currentTime + 0.05);
      return true;
    }
    this.pendingPreset = p;
    return false;
  }

  private applyPreset(p: PresetKey, keepParams: boolean, at: number) {
    const fromScape = SCAPES[this.preset as keyof typeof SCAPES];
    const fromChord = this.scapeChord;
    const outBar = this.spb * 4;

    this.preset = p;
    this.pendingPreset = null;
    this.scapeChord = null;
    this.holdDeg = false;
    if (SCAPES[p as keyof typeof SCAPES] && !keepParams) {
      Object.assign(this.params, SCAPES[p as keyof typeof SCAPES].defaults);
    }
    if (!this.ctx) return;

    if (this.scene && this.playing) {
      this.retireScene(this.scene, at, outBar);
      this.scene = this.newScene();
    }
    // The grid keeps running; the new preset just hears it from its own zero.
    this.stepBase = this.step;
    this.presetStartedAt = at;

    const toScape = SCAPES[p as keyof typeof SCAPES];
    if (toScape && fromScape && fromChord) {
      this.scapeDeg = this.nearestDegree(toScape, fromChord);
      this.holdDeg = true;
    }
    this.applyPresetFx();
  }

  public applyPresetFx() {
    if (!this.ctx || !this.delayNode || !this.reverbSend || !this.vinylGain) return;
    const t = this.ctx.currentTime;
    this.delayNode.delayTime.setTargetAtTime(this.spb * 0.75, t, 0.05);
    const rev = this.isScape()
      ? 0.1 + 0.6 * this.eff('space')
      : { edm: 0.16, ambient: 0.55, lofi: 0.22 }[this.preset as 'edm' | 'ambient' | 'lofi'];
    this.reverbSend.gain.setTargetAtTime(rev, t, 0.2);
    this.setRoom(this.roomTarget(), t);
    this.vinylGain.gain.setTargetAtTime(this.preset === 'lofi' ? 0.012 * this.layers.texture : 0, t, 0.3);
    this.updateTexture(t);
  }

  public updateTexture(t: number) {
    if (!this.ctx || !this.texGain || !this.texLP) return;
    const lvl = this.isScape() ? this.layers.texture : 0;
    this.texGain.gain.setTargetAtTime((0.012 + 0.022 * this.eff('texture')) * lvl, t, 0.8);
    this.texLP.frequency.setTargetAtTime(250 + 2800 * this.eff('brightness'), t, 0.8);
  }

  public setVolume(v: number) {
    this.volume = v;
    if (this.master && this.ctx) {
      this.master.gain.setTargetAtTime(v * v, this.ctx.currentTime, 0.05);
    }
  }

  private schedule() {
    if (!this.ctx) return;
    const ahead = this.ctx.currentTime + 0.18;
    while (this.nextTime < ahead) {
      if (this.pendingPreset && (this.step - this.stepBase) % 16 === 0) {
        this.applyPreset(this.pendingPreset, false, this.nextTime);
      }
      this.playStep(this.step - this.stepBase, this.nextTime);
      this.nextTime += this.spb / 4;
      this.step++;
    }
  }

  private playStep(s: number, t: number) {
    if (this.isScape()) this.stepScape(s, t);
    else if (this.preset === 'edm') this.stepEDM(s, t);
    else if (this.preset === 'ambient') this.stepAmbient(s, t);
    else this.stepLofi(s, t);
  }

  private addEvent(ev: MusicEvent) {
    this.events.push(ev);
    // Cleanup old events if queue grows too large
    if (this.eventHeadIndex > 200) {
      this.events = this.events.slice(this.eventHeadIndex);
      this.eventHeadIndex = 0;
    }
  }

  /* Audio Voice Helpers with automatic node disconnection for fast GC */
  private kick(t: number, fHi: number, fLo: number, dur: number, vol: number, visual = true) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    const o = c.createOscillator();
    const g = c.createGain();
    o.frequency.setValueAtTime(fHi, t);
    o.frequency.exponentialRampToValueAtTime(fLo, t + dur * 0.5);
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.001, t + dur);

    o.connect(g);
    g.connect(S.dry);

    o.onended = () => {
      o.disconnect();
      g.disconnect();
    };

    o.start(t);
    o.stop(t + dur + 0.1);

    if (visual) this.addEvent({ t, type: 'kick' });
  }

  private duckIt(t: number) {
    if (!this.duck) return;
    const g = this.duck.gain;
    g.cancelScheduledValues(t);
    g.setValueAtTime(0.22, t);
    g.linearRampToValueAtTime(1.0, t + 0.3);
  }

  private noiseHit(t: number, filterType: BiquadFilterType, freq: number, q: number, vol: number, dur: number, pan = 0) {
    if (!this.ctx || !this.noiseBuf || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    const src = c.createBufferSource();
    src.buffer = this.noiseBuf;
    src.loop = true;
    src.loopStart = this.rnd() * 0.5;

    const f = c.createBiquadFilter();
    f.type = filterType;
    f.frequency.value = freq;
    f.Q.value = q;

    const g = c.createGain();
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.001, t + dur);

    const out = this.place(g, pan);
    src.connect(f);
    f.connect(g);
    out.connect(S.dry);

    src.onended = () => {
      src.disconnect();
      f.disconnect();
      g.disconnect();
      out.disconnect();
    };

    src.start(t);
    src.stop(t + dur + 0.05);
  }

  private bass(t: number, midi: number, dur: number, vol: number) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    const o = c.createOscillator();
    const f = c.createBiquadFilter();
    const g = c.createGain();

    o.type = 'sawtooth';
    o.frequency.value = N(midi);
    f.type = 'lowpass';
    f.frequency.setValueAtTime(650, t);
    f.frequency.exponentialRampToValueAtTime(220, t + dur);
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.001, t + dur);

    o.connect(f);
    f.connect(g);
    g.connect(S.duck);

    o.onended = () => {
      o.disconnect();
      f.disconnect();
      g.disconnect();
    };

    o.start(t);
    o.stop(t + dur + 0.1);

    this.addEvent({ t, type: 'bass', midi });
  }

  private pad(t: number, chord: number[], dur: number, vol: number, cutoff: number, attack: number, release: number) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    for (let i = 0; i < chord.length; i++) {
      const midi = chord[i];
      // The detune pair goes hard left and right, and each chord tone leans a
      // little further out, so a triad occupies the field instead of one point.
      const lean = chord.length > 1 ? (i / (chord.length - 1) - 0.5) * 0.2 : 0;
      for (const det of [-7, 7]) {
        const o = c.createOscillator();
        const g = c.createGain();
        const f = c.createBiquadFilter();
        o.type = 'sawtooth';
        o.frequency.value = N(midi);
        o.detune.value = det;
        f.type = 'lowpass';
        f.frequency.value = cutoff;

        g.gain.setValueAtTime(0.0001, t);
        g.gain.linearRampToValueAtTime(vol, t + attack);
        g.gain.setValueAtTime(vol, t + Math.max(attack, dur - release));
        g.gain.linearRampToValueAtTime(0.0001, t + dur + 0.05);

        const out = this.place(g, (det / 7) * 0.55 + lean);
        o.connect(f);
        f.connect(g);
        out.connect(S.duck);
        out.connect(S.rev);

        o.onended = () => {
          o.disconnect();
          f.disconnect();
          g.disconnect();
          out.disconnect();
        };

        o.start(t);
        o.stop(t + dur + 0.2);
      }
    }
    this.addEvent({ t, type: 'chord', midi: chord[0] });
  }

  private lead(t: number, midi: number, dur: number, vol: number) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    for (const det of [-9, 9]) {
      const o = c.createOscillator();
      const g = c.createGain();
      const f = c.createBiquadFilter();
      o.type = 'sawtooth';
      o.frequency.value = N(midi);
      o.detune.value = det;
      f.type = 'lowpass';
      f.frequency.value = 2800;

      g.gain.setValueAtTime(0.0001, t);
      g.gain.linearRampToValueAtTime(vol, t + 0.012);
      g.gain.setValueAtTime(vol * 0.8, t + dur * 0.7);
      g.gain.exponentialRampToValueAtTime(0.001, t + dur);

      const out = this.place(g, (det / 9) * 0.42);
      o.connect(f);
      f.connect(g);
      out.connect(S.dry);
      out.connect(S.del);

      o.onended = () => {
        o.disconnect();
        f.disconnect();
        g.disconnect();
        out.disconnect();
      };

      o.start(t);
      o.stop(t + dur + 0.1);
    }
    this.addEvent({ t, type: 'note', midi });
  }

  private pluck(t: number, midi: number, vol: number, dur: number, type: OscillatorType = 'sine', sendRev = true) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    const o = c.createOscillator();
    const g = c.createGain();
    o.type = type;
    o.frequency.value = N(midi);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.linearRampToValueAtTime(vol, t + 0.01);
    g.gain.exponentialRampToValueAtTime(0.001, t + dur);

    // Pitch class maps to stereo position the same way FluidSymphony maps it to
    // x, so a note heard on the left splashes on the left.
    const pan = (((midi % 12) / 11) - 0.5) * 0.9 + (this.rnd() - 0.5) * 0.16;
    const out = this.place(g, pan);
    o.connect(g);
    out.connect(S.dry);
    if (sendRev) out.connect(S.rev);

    o.onended = () => {
      o.disconnect();
      g.disconnect();
      out.disconnect();
    };

    o.start(t);
    o.stop(t + dur + 0.1);

    this.addEvent({ t, type: 'note', midi });
  }

  private rhodes(t: number, midi: number, vol: number, dur: number) {
    if (!this.ctx || !this.scene) return;
    const S = this.scene;
    const c = this.ctx;
    const g = c.createGain();
    const f = c.createBiquadFilter();
    f.type = 'lowpass';
    f.frequency.value = 2000;
    g.gain.setValueAtTime(0.0001, t);
    g.gain.linearRampToValueAtTime(vol, t + 0.006);
    g.gain.exponentialRampToValueAtTime(0.001, t + dur);

    const o1 = c.createOscillator();
    o1.type = 'sine';
    o1.frequency.value = N(midi);
    o1.detune.value = (this.rnd() - 0.5) * 8;

    const o2 = c.createOscillator();
    o2.type = 'sine';
    o2.frequency.value = N(midi) * 2;

    const g2 = c.createGain();
    g2.gain.value = 0.28;

    const out = this.place(g, (((midi % 12) / 11) - 0.5) * 0.5);
    o1.connect(f);
    o2.connect(g2);
    g2.connect(f);
    f.connect(g);
    out.connect(S.dry);
    out.connect(S.rev);

    o1.onended = () => {
      o1.disconnect();
      o2.disconnect();
      g2.disconnect();
      f.disconnect();
      g.disconnect();
      out.disconnect();
    };

    o1.start(t);
    o1.stop(t + dur + 0.1);
    o2.start(t);
    o2.stop(t + dur + 0.1);

    this.addEvent({ t, type: 'note', midi });
  }

  /* Scenes & Presets steps */
  private scapeTriad(sc: { root: number; scale: number[] }, deg: number): number[] {
    const sl = sc.scale;
    const tone = (i: number) =>
      sc.root + 12 * Math.floor((deg + i) / sl.length) + sl[(deg + i) % sl.length];
    return [tone(0), tone(2), tone(4)];
  }

  /** The degree whose triad shares the most pitch classes with the chord that is
   *  still sounding. Two scales that share notes hand over on those notes. */
  private nearestDegree(sc: { root: number; scale: number[] }, from: number[]): number {
    const pcs = new Set(from.map(m => ((m % 12) + 12) % 12));
    let best = 0;
    let bestShared = -1;
    for (let deg = 0; deg < sc.scale.length; deg++) {
      const shared = this.scapeTriad(sc, deg).filter(m => pcs.has(((m % 12) + 12) % 12)).length;
      if (shared > bestShared) {
        bestShared = shared;
        best = deg;
      }
    }
    return best;
  }

  /** Move to a new chord and lay down its pad and sub. Split out of stepScape so a
   *  preset change can enter on a chosen chord rather than wait for the next one. */
  private scapeChordChange(t: number, walk: boolean) {
    const sc = SCAPES[this.preset as keyof typeof SCAPES];
    if (!sc || !this.ctx || !this.scene) return;
    if (walk) {
      const by = [-2, -1, 1, 2][Math.floor(this.rnd() * 4)];
      this.scapeDeg = ((this.scapeDeg || 0) + by + sc.scale.length * 8) % sc.scale.length;
    }
    const chord = (this.scapeChord = this.scapeTriad(sc, this.scapeDeg));

    const bright = this.eff('brightness');
    const pace = this.eff('pace');
    const spb = this.spb;

    const L = this.mix();
    if (L.pads > 0) {
      this.pad(t, chord.map(m => m + 12), spb * 8 + 2, 0.05 * L.pads, 350 + 2200 * bright, 1.0 + 2.5 * (1 - pace), 3.0);
    }
    if (L.bass > 0) {
      const c = this.ctx;
      const o = c.createOscillator();
      const g = c.createGain();
      o.type = 'sine';
      o.frequency.value = N(chord[0] - 12);
      g.gain.setValueAtTime(0.0001, t);
      g.gain.linearRampToValueAtTime((0.1 + 0.1 * (1 - bright)) * L.bass, t + 2.5);
      g.gain.linearRampToValueAtTime(0.0001, t + spb * 8 + 1.5);
      o.connect(g);
      g.connect(this.scene.dry);
      o.onended = () => {
        o.disconnect();
        g.disconnect();
      };
      o.start(t);
      o.stop(t + spb * 8 + 2);
      this.addEvent({ t, type: 'bass', midi: chord[0] });
    }
  }

  private stepScape(s: number, t: number) {
    const sc = SCAPES[this.preset as keyof typeof SCAPES];
    if (!sc) return;
    const L = this.mix();
    const dens = Math.min(1, this.eff('density') * (0.5 + this.energy));
    const bright = this.eff('brightness');
    const pulse = this.eff('pulse');
    const pace = this.eff('pace');

    if (s % 32 === 0 || !this.scapeChord) {
      // After a preset change the degree is already chosen for common tones with
      // the outgoing chord, so the first change of a new scene must not walk away
      // from it.
      this.scapeChordChange(t, !this.holdDeg);
      this.holdDeg = false;
    }
    const chord = this.scapeChord;
    if (!chord) return;

    if (L.drums > 0 && pulse > 0.1 && s % 8 === 0) {
      this.kick(t, 65 + 50 * pulse, 36, 0.45 + 0.5 * (1 - pace), (0.14 + 0.34 * pulse) * L.drums);
    }

    if (L.melody > 0 && s % 2 === 0 && this.rnd() < dens * 0.5) {
      const oct = this.rnd() < 0.45 ? 24 : 12;
      const midi = chord[Math.floor(this.rnd() * 3)] + oct;
      this.pluck(t, midi, (0.028 + 0.05 * dens) * L.melody, 0.7 + 1.6 * (1 - pace), this.rnd() < 0.5 ? 'sine' : 'triangle');
    }
    if (L.melody > 0 && s % 16 === 8 && this.rnd() < 0.18 * bright) {
      this.pluck(t, chord[0] + 36, 0.018 * L.melody, 2.2, 'sine');
    }

    if (s % 16 === 0) {
      this.applyPresetFx();
    }
  }

  private stepEDM(s: number, t: number) {
    const bar = Math.floor(s / 16);
    const sb = s % 16;
    const L = this.mix();
    const sec = Math.floor(bar / 8) % 4;
    const secMult = [0.6, 0.85, 1.0, 0.55][sec];
    const e = this.energy * secMult;
    const ch = this.EDM_CHORDS[bar % 4];
    const spb = this.spb;
    const st = spb / 4;

    if (sb === 0 && L.pads > 0) this.pad(t, ch, spb * 4, 0.05 * L.pads, 1100 + 600 * e, 0.08, 0.6);

    if (L.drums > 0) {
      if (s % 4 === 0 && e >= 0.28) {
        this.kick(t, 150, 44, 0.3, 0.95 * L.drums);
        this.duckIt(t);
      }
      // Hats alternate sides, the clap stays centred, the riser opens wide.
      if (s % 4 === 2 && e >= 0.48) this.noiseHit(t, 'highpass', 7500, 1, 0.16 * L.drums, 0.05, sb % 8 < 4 ? -0.5 : 0.5);
      if ((sb === 4 || sb === 12) && e >= 0.62) this.noiseHit(t, 'bandpass', 1800, 1.2, 0.4 * L.drums, 0.13);
      if (sec === 3 && bar % 8 === 7) this.noiseHit(t, 'bandpass', 1900, 1.5, (0.12 + 0.02 * sb) * L.drums, 0.08, (this.rnd() - 0.5) * 1.4);
    }

    if (L.bass > 0 && s % 2 === 0 && e >= 0.42) {
      this.bass(t, this.EDM_ROOTS[bar % 4], st * 2 * 0.9, (s % 4 === 0 ? 0.3 : 0.22) * L.bass);
    }

    if (L.melody > 0 && e >= 0.38) {
      const ph = s % 32;
      const hit = this.EDM_MOTIF[ph];
      if (hit) {
        this.lead(t, hit[0], st * hit[1] * 0.95, 0.16 * L.melody);
        if (e >= 0.85) this.lead(t, hit[0] + 12, st * hit[1] * 0.95, 0.07 * L.melody);
      }
    }

    if (L.melody > 0 && e >= 0.78) {
      const tones = [ch[0] + 12, ch[1] + 12, ch[2] + 12, ch[1] + 24];
      this.pluck(t, tones[s % 4], 0.07 * L.melody, 0.14, 'square', false);
    }
  }

  private stepAmbient(s: number, t: number) {
    const chordIdx = Math.floor(s / 32) % 4;
    const L = this.mix();
    const ch = this.AMB_CHORDS[chordIdx];
    const e = this.energy;
    const spb = this.spb;

    if (s % 32 === 0) {
      if (L.pads > 0) this.pad(t, ch, spb * 8 + 1, 0.055 * L.pads, 750 + 500 * e, 2.0, 3.0);
      if (L.bass > 0 && this.ctx && this.scene) {
        const c = this.ctx;
        const o = c.createOscillator();
        const g = c.createGain();
        o.type = 'sine';
        o.frequency.value = N(this.AMB_ROOTS[chordIdx] - 12);
        g.gain.setValueAtTime(0.0001, t);
        g.gain.linearRampToValueAtTime((0.16 * e + 0.04) * L.bass, t + 2.5);
        g.gain.linearRampToValueAtTime(0.0001, t + spb * 8);
        o.connect(g);
        g.connect(this.scene.dry);
        o.onended = () => {
          o.disconnect();
          g.disconnect();
        };
        o.start(t);
        o.stop(t + spb * 8 + 0.5);
      }
    }
    if (L.drums > 0 && s % 64 === 16 && e >= 0.35) this.kick(t, 90, 32, 1.1, 0.5 * L.drums);

    if (L.melody > 0 && s % 2 === 0 && this.rnd() < 0.3 + 0.45 * e) {
      const oct = this.rnd() < 0.4 ? 24 : 12;
      const midi = ch[Math.floor(this.rnd() * 3)] + oct;
      this.pluck(t, midi, (0.05 + 0.05 * e) * L.melody, 0.9, this.rnd() < 0.5 ? 'sine' : 'triangle');
    }
    if (L.melody > 0 && s % 16 === 8 && this.rnd() < 0.3 * e) {
      this.pluck(t, ch[0] + 36, 0.025 * L.melody, 1.6, 'sine');
    }
  }

  private stepLofi(s: number, t: number) {
    const bar = Math.floor(s / 16);
    const sb = s % 16;
    const L = this.mix();
    const ch = this.LOFI_CHORDS[bar % 4];
    const e = this.energy;
    const spb = this.spb;
    const swing = s % 4 === 2 ? spb * 0.14 : 0;
    t += swing;

    if ((sb === 0 || sb === 10) && L.pads > 0) {
      for (let i = 0; i < ch.length; i++) {
        this.rhodes(t + i * 0.012, ch[i], 0.055 * L.pads, 1.6);
      }
    }

    if (L.drums > 0) {
      if ((sb === 0 || sb === 7) && e >= 0.3) this.kick(t, 95, 42, 0.22, 0.5 * L.drums);
      if ((sb === 4 || sb === 12) && e >= 0.3) {
        this.noiseHit(t, 'lowpass', 3800, 0.8, 0.18 * L.drums, 0.16);
        this.addEvent({ t, type: 'bass', midi: 50 });
      }
      if (s % 4 === 2 && e >= 0.55) this.noiseHit(t, 'highpass', 8200, 1, 0.05 * L.drums, 0.04, sb % 8 < 4 ? -0.45 : 0.45);
    }

    if (L.bass > 0 && (sb === 0 || sb === 8) && e >= 0.35) {
      this.bass(t, this.LOFI_ROOTS[bar % 4] - 12, 0.5, 0.2 * L.bass);
    }

    if (L.melody > 0 && s % 4 === 0 && this.rnd() < 0.22 + 0.3 * e) {
      const midi = this.LOFI_PENTA[Math.floor(this.rnd() * 5)];
      this.rhodes(t + 0.02, midi, 0.05 * L.melody, 1.1);
    }
    if (L.texture > 0 && this.rnd() < 0.045) {
      this.noiseHit(t, 'bandpass', 3000, 8, 0.04 * L.texture, 0.015, (this.rnd() - 0.5) * 1.6);
    }
  }

  public consumeMusicEvents(cb: (ev: MusicEvent) => void) {
    if (!this.ctx) return;
    const now = this.ctx.currentTime + 0.03;
    while (this.eventHeadIndex < this.events.length) {
      const ev = this.events[this.eventHeadIndex];
      if (ev.t > now) break;
      this.eventHeadIndex++;
      if (ev.t >= now - 0.5) {
        cb(ev);
      }
    }
  }

  public destroy() {
    this.stop();
    if (this.vinylSource) {
      try {
        this.vinylSource.stop();
      } catch (error) {
        if (!(error instanceof DOMException) || error.name !== 'InvalidStateError') {
          console.warn('Could not stop the Soundscape vinyl source', error);
        }
      }
      this.vinylSource.disconnect();
    }
    if (this.texSource) {
      try {
        this.texSource.stop();
      } catch (error) {
        if (!(error instanceof DOMException) || error.name !== 'InvalidStateError') {
          console.warn('Could not stop the Soundscape texture source', error);
        }
      }
      this.texSource.disconnect();
    }
    if (this.ctx) {
      this.ctx.close();
      this.ctx = null;
    }
  }
}
