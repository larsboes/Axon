/**
 * The app's way to the Mac. Inside the Tauri bundle a relative `fetch` resolves
 * against the app's own origin and reaches nothing, so `request()` in `./api.ts`
 * hands Mac paths to the native `mac_request` command instead
 * (`src-tauri/src/mac_bridge.rs`, which states why the call leaves from Rust).
 *
 * The Mac's address is not in this file or in any other: it is a house fact and
 * the repository is public. The operator sets it in the app, and the app keeps it
 * in its own data directory.
 */
import { invoke } from '@tauri-apps/api/core';

/** Mirrors `NOT_CONFIGURED` in `src-tauri/src/mac_bridge.rs`. */
export const NOT_CONFIGURED = 'mac-bridge: not configured';

/** The path the settings panel calls to test the address. It is on the allow-list. */
export const HEALTH_PATH = '/axon-status/api/axon-status/health';

export interface MacResponse {
  status: number;
  content_type: string | null;
  body: string;
}

export interface MacSettings {
  base_url: string | null;
}

export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The headers the Rust side forwards. Anything else is not sent across. */
function headerPairs(headers: HeadersInit | undefined): [string, string][] {
  if (!headers) return [];
  if (headers instanceof Headers) return [...headers.entries()];
  if (Array.isArray(headers)) return headers.map(([k, v]) => [k, v] as [string, string]);
  return Object.entries(headers);
}

/**
 * Sends one request to the Mac. Rejects with the native error string when the
 * request never got an answer (no address set, refused path, network down).
 */
export async function macRequest(path: string, init?: RequestInit): Promise<MacResponse> {
  const body = init?.body;
  if (body != null && typeof body !== 'string') {
    throw new Error('mac-bridge: only a text body can be sent to the Mac');
  }
  return invoke<MacResponse>('mac_request', {
    request: {
      method: (init?.method ?? 'GET').toUpperCase(),
      path,
      body: body ?? null,
      headers: headerPairs(init?.headers),
    },
  });
}

/** One binary answer from the Mac, as `mac_request_bytes` returns it. */
export interface MacBytes {
  status: number;
  contentType: string | null;
  bytes: Uint8Array;
}

/**
 * Reads the frame `frame_bytes` in `src-tauri/src/mac_bridge.rs` writes:
 * status (u16 BE), content-type length (u16 BE), content-type, body.
 */
export function decodeBytesFrame(raw: ArrayBuffer | ArrayBufferView | number[]): MacBytes {
  const all =
    raw instanceof ArrayBuffer
      ? new Uint8Array(raw)
      : ArrayBuffer.isView(raw)
        ? new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength)
        : Uint8Array.from(raw);
  if (all.length < 4) throw new Error('mac-bridge: the binary answer is shorter than its header');
  const status = (all[0] << 8) | all[1];
  const typeLength = (all[2] << 8) | all[3];
  if (all.length < 4 + typeLength) throw new Error('mac-bridge: the binary answer is cut short');
  const type = new TextDecoder().decode(all.subarray(4, 4 + typeLength));
  return { status, contentType: type || null, bytes: all.subarray(4 + typeLength) };
}

/**
 * Fetches one Mac path as bytes (GET only). The native side refuses an answer
 * above 64 MiB. Rejects with the native error string, as `macRequest` does.
 */
export async function macRequestBytes(path: string): Promise<MacBytes> {
  const raw = await invoke<ArrayBuffer | ArrayBufferView | number[]>('mac_request_bytes', {
    path,
    headers: null,
  });
  return decodeBytesFrame(raw);
}

export function getMacSettings(): Promise<MacSettings> {
  return invoke<MacSettings>('mac_settings_get');
}

export function setMacBaseUrl(baseUrl: string): Promise<MacSettings> {
  return invoke<MacSettings>('mac_settings_set', { baseUrl });
}

/** The error text of a failed `invoke`, which Tauri rejects with as a bare string. */
export function bridgeErrorText(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
