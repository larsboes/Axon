import { afterEach, describe, expect, mock, test } from 'bun:test';
import {
  acquireBridgedUrl,
  bridgedSrc,
  bridgedUrlCacheSize,
  releaseBridgedUrl,
} from '../src/lib/bridged-url';
import { decodeBytesFrame } from '../src/lib/mac-bridge';

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

const g = globalThis as unknown as { window?: Record<string, unknown> };
const realCreate = URL.createObjectURL;
const realRevoke = URL.revokeObjectURL;

function enterTauri(invoke: Invoke) {
  g.window = { __TAURI_INTERNALS__: { invoke, transformCallback: () => 0 } };
}

/** The frame `frame_bytes` in `src-tauri/src/mac_bridge.rs` writes. */
function frame(status: number, type: string, body: Uint8Array): ArrayBuffer {
  const t = new TextEncoder().encode(type);
  const out = new Uint8Array(4 + t.length + body.length);
  out[0] = status >> 8;
  out[1] = status & 0xff;
  out[2] = t.length >> 8;
  out[3] = t.length & 0xff;
  out.set(t, 4);
  out.set(body, 4 + t.length);
  return out.buffer;
}

afterEach(() => {
  delete g.window;
  URL.createObjectURL = realCreate;
  URL.revokeObjectURL = realRevoke;
});

describe('decodeBytesFrame', () => {
  test('reads status, content type and the raw body', () => {
    const body = new Uint8Array([0, 0xff, 0x80, 10]);
    const decoded = decodeBytesFrame(frame(200, 'image/jpeg', body));
    expect(decoded.status).toBe(200);
    expect(decoded.contentType).toBe('image/jpeg');
    expect([...decoded.bytes]).toEqual([0, 0xff, 0x80, 10]);
  });

  test('an empty content type is null, and a short frame is refused', () => {
    expect(decodeBytesFrame(frame(404, '', new Uint8Array())).contentType).toBeNull();
    expect(() => decodeBytesFrame(new Uint8Array([0, 200]))).toThrow();
  });
});

describe('acquireBridgedUrl', () => {
  test('returns the path unchanged in the browser', async () => {
    expect(await acquireBridgedUrl('/interior/api/media/a.jpg')).toBe('/interior/api/media/a.jpg');
    expect(bridgedUrlCacheSize()).toBe(0);
  });

  test('in Tauri, builds a blob URL from the bridge bytes, caches it, and revokes on last release', async () => {
    const invoke = mock<Invoke>(async () => frame(200, 'image/png', new Uint8Array([1, 2, 3])));
    enterTauri(invoke);
    const blobs: Blob[] = [];
    URL.createObjectURL = ((blob: Blob) => {
      blobs.push(blob);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    const revoke = mock((_url: string) => {});
    URL.revokeObjectURL = revoke;

    const path = '/interior/api/media/lamp.png';
    const [a, b] = await Promise.all([acquireBridgedUrl(path), acquireBridgedUrl(path)]);
    expect(a).toBe('blob:test/1');
    expect(b).toBe(a);
    expect(invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = invoke.mock.calls[0];
    expect(cmd).toBe('mac_request_bytes');
    expect(args).toEqual({ path, headers: null });
    expect(blobs[0].type).toBe('image/png');
    expect([...new Uint8Array(await blobs[0].arrayBuffer())]).toEqual([1, 2, 3]);

    releaseBridgedUrl(path);
    await Promise.resolve();
    expect(revoke).not.toHaveBeenCalled();
    releaseBridgedUrl(path);
    await new Promise((r) => setTimeout(r, 0));
    expect(revoke).toHaveBeenCalledWith('blob:test/1');
    expect(bridgedUrlCacheSize()).toBe(0);
  });

  test('in Tauri, an error status rejects and is not cached', async () => {
    const invoke = mock<Invoke>(async () => frame(404, 'application/json', new Uint8Array()));
    enterTauri(invoke);
    await expect(acquireBridgedUrl('/interior/api/media/gone.jpg')).rejects.toThrow('404');
    await new Promise((r) => setTimeout(r, 0));
    expect(bridgedUrlCacheSize()).toBe(0);
  });

  test('in Tauri, an absolute URL is not sent through the bridge', async () => {
    const invoke = mock<Invoke>(async () => frame(200, '', new Uint8Array()));
    enterTauri(invoke);
    expect(await acquireBridgedUrl('https://example.org/x.jpg')).toBe('https://example.org/x.jpg');
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe('bridgedSrc action', () => {
  test('sets the path directly in the browser', () => {
    const node = { src: '', removeAttribute: () => {} } as unknown as HTMLImageElement;
    const action = bridgedSrc(node, '/trips/api/media/x.jpg');
    expect(node.src).toBe('/trips/api/media/x.jpg');
    action.destroy();
  });

  test('sets a blob URL in Tauri and releases it on destroy', async () => {
    enterTauri(async () => frame(200, 'image/jpeg', new Uint8Array([9])));
    URL.createObjectURL = (() => 'blob:test/img') as typeof URL.createObjectURL;
    const revoke = mock((_url: string) => {});
    URL.revokeObjectURL = revoke;
    const node = { src: '', removeAttribute: () => {} } as unknown as HTMLImageElement;
    const action = bridgedSrc(node, '/interior/api/media/y.jpg');
    await new Promise((r) => setTimeout(r, 0));
    expect(node.src).toBe('blob:test/img');
    action.destroy();
    await new Promise((r) => setTimeout(r, 0));
    expect(revoke).toHaveBeenCalledWith('blob:test/img');
  });
});
