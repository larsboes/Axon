import type { PresetKey, AudioLayers, AudioParams, SoundscapeSession } from './types';

/**
 * The conductor's `Scape`, field for field (capabilities/soundscape/src/main.rs).
 * Two declarations of one shape is one too many; they stay equal by hand until the
 * param shape moves into schemas/ and both sides read it.
 */
export interface Scape {
  preset: PresetKey;
  params: AudioParams;
  layers: AudioLayers;
  seed: number;
  playing: boolean;
  volume: number;
  energy: number;
  session: SoundscapeSession | null;
}

/** Send what changed, never the whole state — a full PUT lets a stale client undo another's edit. */
export type Patch = Partial<Omit<Scape, 'session'>> & { session?: SoundscapeSession | null };

/**
 * The client holding the audio output. Holding it is not the same as playing:
 * a paused tab keeps the output so a remote play has something to resume.
 */
export interface Host {
  id: string;
  label: string;
  since_ms: number;
}

/** State as the conductor reports it: the scape, plus who can actually sound it. */
export type StateView = Scape & { host: Host | null };

/** What arrives on the stream: the new state, plus which client caused it. */
type Change = StateView & { origin: string | null };

/** Well inside the conductor's 15s TTL, so one missed beat is not a lost claim. */
const HEARTBEAT_MS = 5000;

/** Same origin as the page: the conductor serves this bundle itself. */
const API = '/api/soundscape';

/**
 * Sliders fire per pixel. Posting each one would spend a request on a value that is
 * already stale by the time it lands, so edits coalesce into one patch per window.
 */
const COALESCE_MS = 120;

export interface ConductorHandlers {
  /** A state the conductor holds and this client did not cause. */
  onState: (view: StateView) => void;
  /** Whether the conductor is currently reachable. */
  onReachable?: (reachable: boolean) => void;
}

/** Either this client now holds the output, or someone else does. */
export type ClaimResult = { held: true } | { held: false; host: Host | null };

/**
 * The browser's half of the state contract: it owns the audio, the conductor owns
 * the answer to what should be playing. Deliberately framework-free — the store
 * holds the reactive state, this holds the wire.
 */
export class Conductor {
  /**
   * Identifies this client's own edits so they can be dropped when they come back
   * on the stream. Without it, every slider fights its own echo.
   */
  readonly id: string = typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `c${Math.random().toString(36).slice(2)}`;

  private stream: EventSource | null = null;
  private handlers: ConductorHandlers | null = null;
  private pending: Patch | null = null;
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private reachable = true;

  /**
   * The stream opens with the current state, so a client that connects mid-session
   * knows where it is without a separate GET.
   */
  connect(handlers: ConductorHandlers) {
    if (typeof window === 'undefined') return;
    this.handlers = handlers;
    this.stream?.close();

    const stream = new EventSource(`${API}/stream`);
    this.stream = stream;

    stream.addEventListener('state', (event) => {
      let change: Change;
      try {
        change = JSON.parse((event as MessageEvent<string>).data);
      } catch {
        // A frame we cannot read is a frame we skip; the next one carries the
        // whole state anyway, so there is nothing to reconstruct.
        return;
      }
      this.setReachable(true);
      // Our own edit, already applied locally before it was sent.
      if (change.origin === this.id) return;
      const { origin: _origin, ...view } = change;
      this.handlers?.onState(view);
    });

    // EventSource reconnects on its own; what it does not do is tell the UI that
    // the gap exists.
    stream.addEventListener('error', () => this.setReachable(false));
    stream.addEventListener('open', () => this.setReachable(true));
  }

  /**
   * Claim the audio output for this client, or refresh a claim it already holds.
   * `takeover` is the deliberate second attempt after a refusal — never the first,
   * because silently stealing the output from another tab is how two of them end
   * up playing over each other.
   */
  async claimHost(label: string, takeover = false): Promise<ClaimResult> {
    try {
      const response = await fetch(`${API}/host/claim`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: this.id, label, takeover }),
      });
      if (response.ok) {
        this.startHeartbeat(label);
        return { held: true };
      }
      if (response.status === 409) {
        const body = await response.json().catch(() => ({}));
        return { held: false, host: body.host ?? null };
      }
      console.error('[soundscape] claim failed', response.status, await response.text());
      return { held: false, host: null };
    } catch (err) {
      // Unreachable conductor: play locally rather than refuse to make sound
      // because the bookkeeper is down.
      console.error('[soundscape] cannot claim output', err);
      this.setReachable(false);
      return { held: true };
    }
  }

  /** Hand the output back, so the next tab that wants it is not told to take over. */
  async releaseHost() {
    this.stopHeartbeat();
    try {
      await fetch(`${API}/host/release`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: this.id }),
      });
    } catch (err) {
      // The TTL collects it either way; this only makes it immediate.
      console.error('[soundscape] cannot release output', err);
    }
  }

  private startHeartbeat(label: string) {
    if (this.heartbeat !== null) return;
    this.heartbeat = setInterval(() => {
      void fetch(`${API}/host/claim`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: this.id, label, takeover: false }),
      }).catch(() => {
        // A missed beat is not a lost claim; the TTL is three beats wide.
      });
    }, HEARTBEAT_MS);
  }

  private stopHeartbeat() {
    if (this.heartbeat === null) return;
    clearInterval(this.heartbeat);
    this.heartbeat = null;
  }

  disconnect() {
    this.stopHeartbeat();
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    // Whatever was still coalescing is a real edit the operator made; dropping it
    // silently would lose the last thing they touched before navigating away.
    if (this.pending) void this.flush();
    this.stream?.close();
    this.stream = null;
    this.handlers = null;
  }

  /** Queue a change for the conductor. Later fields win within one window. */
  push(patch: Patch) {
    this.pending = { ...this.pending, ...patch };
    if (this.flushTimer !== null) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      void this.flush();
    }, COALESCE_MS);
  }

  private async flush() {
    const patch = this.pending;
    this.pending = null;
    if (!patch) return;

    try {
      const response = await fetch(`${API}/state`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ ...patch, origin: this.id }),
      });
      // A rejected patch means the two sides disagree about what is valid, which
      // is a contract bug and not something to swallow.
      if (!response.ok) {
        console.error('[soundscape] conductor rejected patch', response.status, await response.text());
      }
      this.setReachable(response.ok);
    } catch (err) {
      console.error('[soundscape] conductor unreachable', err);
      this.setReachable(false);
    }
  }

  private setReachable(next: boolean) {
    if (next === this.reachable) return;
    this.reachable = next;
    this.handlers?.onReachable?.(next);
  }
}
