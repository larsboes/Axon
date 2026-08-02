import type { PresetKey, LayerKey, AudioLayers, AudioParams, SoundscapeSession } from './types';
import { AudioEngine, PRESETS } from './audioEngine';
import { Conductor, type Host, type Patch, type StateView } from './conductor';
import { scenarioLabel, type ScenarioInfo } from './scenarios';

class SymphonyStore {
  public isPlaying = $state(false);
  public preset = $state<PresetKey>('edm');
  /** Queued for the next bar; selecting it again switches immediately. */
  public pending = $state<PresetKey | null>(null);
  public volume = $state(0.65);
  public energy = $state(0.7);
  public nowPlayingText = $state('CLASSICAL × EDM · 126 BPM');
  public hasStarted = $state(false);
  public session = $state<SoundscapeSession | null>(null);
  public sessionRemainingMs = $state(0);

  /**
   * Mirrors of the engine's own fields. They live here as well because the
   * conductor sends whole states, and a surface has to be able to render what
   * another surface set without reaching into the engine.
   */
  public params = $state<AudioParams>({
    pace: 0.5,
    density: 0.5,
    brightness: 0.5,
    space: 0.5,
    pulse: 0.4,
    texture: 0.4,
  });
  public layers = $state<AudioLayers>({ drums: 1, bass: 1, pads: 1, melody: 1, texture: 1 });
  public seed = $state(0);

  /** False while the conductor is unreachable: local audio keeps playing, nothing syncs. */
  public connected = $state(true);

  /** Which client holds the audio output, if any. Null means nothing can be heard. */
  public host = $state<Host | null>(null);
  /**
   * Set when this surface wanted to play and another one already holds the output.
   * The UI turns this into a question; nothing is taken without an answer.
   */
  public contested = $state<Host | null>(null);

  private audioEngine: AudioEngine | null = null;
  private npTimer: number | null = null;
  private conductor = new Conductor();
  /** How this client identifies itself when it holds the output. */
  private label = 'panel';
  /** True while a conductor state is being applied, so echoes are not sent back. */
  private applying = false;

  /** True when this surface is the one that would make the sound. */
  public get isHost(): boolean {
    return this.host !== null && this.host.id === this.conductor.id;
  }

  /** What is audible right now, as opposed to what is meant to be playing. */
  public get sounding(): boolean {
    return this.isPlaying && this.host !== null;
  }

  public getAudioEngine(): AudioEngine {
    if (!this.audioEngine) {
      this.audioEngine = new AudioEngine();
      this.audioEngine.setPreset(this.preset);
      this.audioEngine.setVolume(this.volume);
      this.audioEngine.energy = this.energy;
      this.seed = this.audioEngine.seed;

      // Update text periodically
      if (typeof window !== 'undefined') {
        this.npTimer = window.setInterval(() => this.tick(), 1000);
      }
    }
    return this.audioEngine;
  }

  /**
   * Join the conductor: adopt what it holds, then follow it. Explicit rather than
   * automatic on construction, because this module also loads during prerender
   * where there is no server to talk to.
   */
  public connect(label = 'panel') {
    this.label = label;
    this.conductor.connect({
      onState: (view) => this.applyRemote(view),
      onReachable: (reachable) => {
        this.connected = reachable;
      },
    });
  }

  public disconnect() {
    // Hand the output back before leaving, so the next surface is not told to
    // take over from a tab that no longer exists.
    if (this.isHost) void this.conductor.releaseHost();
    this.conductor.disconnect();
  }

  /**
   * Adopt a state someone else set. Audio is the one thing this does not do on its
   * own: a remote play resumes a tab whose context a gesture already unlocked, and
   * never wakes a silent one — browsers forbid it and it would be a surprise.
   */
  private applyRemote(view: StateView) {
    const { host, ...scape } = view;
    const engine = this.getAudioEngine();
    this.applying = true;
    try {
      const wasHost = this.isHost;
      this.host = host;
      // Another surface took the output. Going quiet is the whole point of the
      // handover: two tabs playing the same scape over each other is the defect.
      if (wasHost && !this.isHost && this.isPlaying) {
        engine.stop();
        this.isPlaying = false;
      }

      if (scape.preset !== engine.preset) {
        // Normal preset semantics: while playing this queues for the next bar
        // rather than cutting, so a remote change does not break the beat.
        if (engine.setPreset(scape.preset)) this.preset = engine.preset;
        this.pending = engine.pendingPreset;
      }

      Object.assign(engine.params, scape.params);
      this.params = { ...scape.params };

      Object.assign(engine.layers, scape.layers);
      this.layers = { ...scape.layers };
      if (engine.ctx) engine.applyPresetFx();

      if (scape.seed !== engine.seed) {
        engine.setSeed(scape.seed);
        this.seed = engine.seed;
      }

      if (scape.volume !== this.volume) {
        this.volume = scape.volume;
        engine.setVolume(scape.volume);
      }
      if (scape.energy !== this.energy) {
        this.energy = scape.energy;
        engine.energy = scape.energy;
      }

      const hadSession = this.session !== null;
      this.session = scape.session ?? null;
      this.updateSessionRemaining();
      if (hadSession && !this.session && engine.ctx) engine.setVolume(this.volume);

      // Only the surface holding the output acts on play/pause. Everyone else
      // renders the intent — a remote play resumes the tab that has the sound,
      // it does not wake a silent one, which browsers forbid anyway.
      if (scape.playing !== this.isPlaying && this.isHost && this.hasStarted) {
        if (scape.playing) engine.start();
        else engine.stop();
        this.isPlaying = scape.playing;
      }

      this.updateNowPlaying();
    } finally {
      this.applying = false;
    }
  }

  /** Tell the conductor what changed here — unless the change came from there. */
  private push(patch: Patch) {
    if (this.applying) return;
    this.conductor.push(patch);
  }

  public updateNowPlaying() {
    if (!this.audioEngine) return;
    // The engine owns when a queued preset lands, so read it back rather than
    // assume the last click took effect.
    this.preset = this.audioEngine.preset;
    this.pending = this.audioEngine.pendingPreset;
    const pr = PRESETS[this.preset];
    let txt = pr.label;
    if (this.audioEngine.isScape()) {
      const h = new Date().getHours();
      const tod = h < 5 ? 'night' : h < 9 ? 'dawn' : h < 17 ? 'day' : h < 22 ? 'dusk' : 'night';
      txt += ' · ' + Math.round(this.audioEngine.bpm) + ' BPM · ' + tod;
      if (this.audioEngine.arcPhase) txt += ' · ' + this.audioEngine.arcPhase;
    }
    if (this.session) txt += ' · ' + this.sessionClock;
    this.nowPlayingText = txt;
  }

  public get sessionClock(): string {
    const seconds = Math.max(0, Math.ceil(this.sessionRemainingMs / 1000));
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
  }

  public get sessionLabel(): string | null {
    return this.session ? scenarioLabel(this.session.scenario) : null;
  }

  private updateSessionRemaining(now = Date.now()) {
    if (!this.session) {
      this.sessionRemainingMs = 0;
      return;
    }
    const running = this.session.running_since_ms === null ? 0 : Math.max(0, now - this.session.running_since_ms);
    this.sessionRemainingMs = Math.max(0, this.session.duration_ms - this.session.elapsed_ms - running);
  }

  private tick() {
    this.updateSessionRemaining();
    if (
      this.session &&
      this.sessionRemainingMs > 0 &&
      this.sessionRemainingMs < 60_000 &&
      this.isHost &&
      this.isPlaying &&
      this.audioEngine?.ctx &&
      this.audioEngine.master
    ) {
      const fade = this.sessionRemainingMs / 60_000;
      this.audioEngine.master.gain.setTargetAtTime(
        this.volume * this.volume * fade,
        this.audioEngine.ctx.currentTime,
        0.4,
      );
    }
    if (this.session && this.sessionRemainingMs <= 0) this.finishSession();
    this.updateNowPlaying();
  }

  private pausedSession(now = Date.now()): SoundscapeSession | null {
    if (!this.session || this.session.running_since_ms === null) return this.session;
    return {
      ...this.session,
      elapsed_ms: Math.min(
        this.session.duration_ms,
        this.session.elapsed_ms + Math.max(0, now - this.session.running_since_ms),
      ),
      running_since_ms: null,
    };
  }

  private runningSession(now = Date.now()): SoundscapeSession | null {
    if (!this.session || this.session.running_since_ms !== null) return this.session;
    return { ...this.session, running_since_ms: now };
  }

  private finishSession() {
    const engine = this.getAudioEngine();
    this.session = null;
    this.sessionRemainingMs = 0;
    if (this.isPlaying) engine.stop();
    this.isPlaying = false;
    engine.setVolume(this.volume);
    this.push({ session: null, playing: false });
  }

  public async togglePlay() {
    const engine = this.getAudioEngine();
    if (this.isPlaying) {
      engine.stop();
      this.isPlaying = false;
      this.session = this.pausedSession();
      this.updateSessionRemaining();
      // The output stays claimed while paused: it is this tab's speaker either
      // way, and releasing it would leave a remote play with nothing to resume.
      this.push({ playing: false, ...(this.session ? { session: this.session } : {}) });
      this.updateNowPlaying();
      return;
    }

    // Build the context inside the gesture that asked for sound — after an await
    // the activation is gone and the browser refuses to start the audio.
    engine.init();

    const claim = await this.conductor.claimHost(this.label);
    if (!claim.held) {
      this.contested = claim.host;
      return;
    }

    this.contested = null;
    engine.start();
    this.isPlaying = true;
    this.hasStarted = true;
    this.session = this.runningSession();
    this.updateSessionRemaining();
    this.push({ playing: true, ...(this.session ? { session: this.session } : {}) });
    this.updateNowPlaying();
  }

  /** Answer to a contested output: take it, and let the other surface fall silent. */
  public async takeOver() {
    const engine = this.getAudioEngine();
    engine.init();
    const claim = await this.conductor.claimHost(this.label, true);
    if (!claim.held) return;

    this.contested = null;
    engine.start();
    this.isPlaying = true;
    this.hasStarted = true;
    this.session = this.runningSession();
    this.updateSessionRemaining();
    this.push({ playing: true, ...(this.session ? { session: this.session } : {}) });
    this.updateNowPlaying();
  }

  /** Drop the question without taking anything. */
  public dismissContest() {
    this.contested = null;
  }

  /** True when the switch landed now, false when it is queued for the next bar. */
  public setPreset(p: PresetKey): boolean {
    const engine = this.getAudioEngine();
    const landed = engine.setPreset(p);
    if (landed) {
      this.preset = engine.preset;
      // A preset carries its own defaults, so the params moved too.
      this.params = { ...engine.params };
    }
    this.pending = engine.pendingPreset;
    if (this.session) {
      this.session = null;
      this.sessionRemainingMs = 0;
    }
    this.push({ preset: p, session: null, ...(landed ? { params: { ...engine.params } } : {}) });
    this.updateNowPlaying();
    return landed;
  }

  public setVolume(v: number) {
    if (v === this.volume) return;
    this.volume = v;
    const engine = this.getAudioEngine();
    engine.setVolume(v);
    this.push({ volume: v });
  }

  public setEnergy(e: number) {
    if (e === this.energy) return;
    this.energy = e;
    const engine = this.getAudioEngine();
    engine.energy = e;
    this.push({ energy: e });
  }

  public async startScenario(scenario: ScenarioInfo) {
    this.setPreset(scenario.preset);
    this.setEnergy(scenario.energy);
    this.session = {
      scenario: scenario.key,
      duration_ms: scenario.minutes * 60_000,
      elapsed_ms: 0,
      running_since_ms: this.isPlaying ? Date.now() : null,
    };
    this.updateSessionRemaining();
    this.push({ session: this.session });
    this.updateNowPlaying();
    if (!this.isPlaying) await this.togglePlay();
  }

  public setTimer(minutes: number | null) {
    if (!minutes) {
      this.cancelSession();
      return;
    }
    this.session = {
      scenario: 'timer',
      duration_ms: minutes * 60_000,
      elapsed_ms: 0,
      running_since_ms: this.isPlaying ? Date.now() : null,
    };
    this.updateSessionRemaining();
    this.push({ session: this.session });
    this.updateNowPlaying();
  }

  public cancelSession() {
    if (!this.session) return;
    this.session = null;
    this.sessionRemainingMs = 0;
    this.getAudioEngine().setVolume(this.volume);
    this.push({ session: null });
    this.updateNowPlaying();
  }

  public setParam(key: keyof AudioParams, value: number) {
    const engine = this.getAudioEngine();
    this.params[key] = value;
    engine.params[key] = value;
    this.push({ params: { ...this.params } });
  }

  public setLayer(key: LayerKey, level: number) {
    const engine = this.getAudioEngine();
    this.layers[key] = level;
    engine.layers[key] = level;
    if (engine.ctx) engine.applyPresetFx();
    this.push({ layers: { ...this.layers } });
  }

  public setSeed(seed: number) {
    const engine = this.getAudioEngine();
    engine.setSeed(seed);
    this.seed = engine.seed;
    this.push({ seed: this.seed });
  }

  public reseed(): number {
    const engine = this.getAudioEngine();
    this.seed = engine.reseed();
    this.push({ seed: this.seed });
    return this.seed;
  }

  public destroy() {
    this.conductor.disconnect();
    if (this.npTimer !== null) {
      clearInterval(this.npTimer);
      this.npTimer = null;
    }
    if (this.audioEngine) {
      this.audioEngine.destroy();
      this.audioEngine = null;
    }
  }
}

export const symphonyStore = new SymphonyStore();
