/**
 * A usable URL for a same-origin capability path, such as a picture under
 * `/interior/api/media/` or the RoomPlan USDZ.
 *
 * In the browser the path itself works, so it comes back unchanged. In the
 * Tauri app a relative path reaches nothing (see `./mac-bridge.ts`), so the
 * bytes come through the native bridge and the caller gets a `blob:` URL.
 *
 * One object URL per path, shared by every holder. Each `acquireBridgedUrl`
 * needs one `releaseBridgedUrl`; the last release revokes the URL.
 */
import { inTauri, macRequestBytes } from './mac-bridge';

interface Entry {
  holders: number;
  url: Promise<string>;
}

const cache = new Map<string, Entry>();

/** True for a path the bridge must carry: relative to the Mac, not protocol-relative. */
function isMacPath(path: string): boolean {
  return path.startsWith('/') && !path.startsWith('//');
}

async function load(path: string): Promise<string> {
  const answer = await macRequestBytes(path);
  if (answer.status < 200 || answer.status >= 300) {
    throw new Error(`mac-bridge: ${path} answered ${answer.status}`);
  }
  // Copy into a fresh ArrayBuffer: a view on a larger buffer would put the
  // frame header into the blob.
  const body = answer.bytes.slice();
  const blob = new Blob([body.buffer as ArrayBuffer], {
    type: answer.contentType ?? 'application/octet-stream',
  });
  return URL.createObjectURL(blob);
}

/** Returns a URL for `path`. In Tauri, call `releaseBridgedUrl(path)` once when done. */
export function acquireBridgedUrl(path: string): Promise<string> {
  if (!inTauri() || !isMacPath(path)) return Promise.resolve(path);
  let entry = cache.get(path);
  if (!entry) {
    const url = load(path);
    entry = { holders: 0, url };
    const created = entry;
    // A failed load is not cached, so the next holder tries again.
    url.catch(() => {
      if (cache.get(path) === created) cache.delete(path);
    });
    cache.set(path, entry);
  }
  entry.holders += 1;
  return entry.url;
}

/** Gives back one hold on `path`. The last one revokes the object URL. */
export function releaseBridgedUrl(path: string): void {
  const entry = cache.get(path);
  if (!entry) return;
  entry.holders -= 1;
  if (entry.holders > 0) return;
  cache.delete(path);
  entry.url.then((url) => URL.revokeObjectURL(url)).catch(() => {});
}

/**
 * Svelte action: `<img use:bridgedSrc={path} />`. Sets `src` to the path in
 * the browser and to a bridged `blob:` URL in the app. Releases on unmount.
 */
export function bridgedSrc(node: HTMLImageElement, path: string | null | undefined) {
  let current: string | null = null;

  function set(next: string | null | undefined) {
    if (next === current) return;
    if (current !== null) releaseBridgedUrl(current);
    current = next ?? null;
    if (current === null) {
      node.removeAttribute('src');
      return;
    }
    const wanted = current;
    if (!inTauri() || !isMacPath(wanted)) {
      node.src = wanted;
      return;
    }
    void acquireBridgedUrl(wanted).then(
      (url) => {
        if (current === wanted) node.src = url;
      },
      () => {
        if (current === wanted) node.removeAttribute('src');
      },
    );
  }

  set(path);
  return {
    update: set,
    destroy() {
      if (current !== null) releaseBridgedUrl(current);
      current = null;
    },
  };
}

/** For tests: the number of paths with a live object URL. */
export function bridgedUrlCacheSize(): number {
  return cache.size;
}
