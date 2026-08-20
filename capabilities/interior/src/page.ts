/**
 * Renders the model as one self-contained HTML page: the layouts as floor plans, and the
 * photographs as clickable camera positions standing on those plans.
 *
 * Self-contained is a hard requirement, not a preference — no external stylesheet, font, script
 * or image host. Everything is inline or a data URI, so the page works from a file:// URL, from
 * a phone with no signal, and inside a viewer with a strict content policy.
 *
 * Nothing here knows a dimension. Every number on the page arrives from the model at runtime,
 * and the SVG is generated from the same coordinates the checker judges, so the picture and the
 * verdict cannot drift apart. That drift is the whole reason this file exists instead of a
 * hand-written page.
 */

import { checkLayout, type CheckResult } from "./clearance.ts";
import { footprint, kindOf, loadFurniture, loadLayout, loadRoom } from "./model.ts";
import type { FurnitureItem, Layout, Room } from "./model.ts";
import { LAYOUTS_DIR, MODEL_DIR } from "./paths.ts";
import { readdir } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";

const MAX_PHOTO_PX = 1400;

export type Camera = {
  datei: string;
  titel: string;
  pos: [number, number];
  blick: number;
  geschaetzt?: boolean;
  zeigt?: string[];
};

/** Downscale through sips when it is there, embed the original when it is not. */
async function photoDataUri(abs: string): Promise<{ uri: string; bytes: number } | null> {
  const file = Bun.file(abs);
  if (!(await file.exists())) return null;

  let bytes = await file.bytes();
  const tmp = join(process.env.TMPDIR ?? "/tmp", `interior-${basename(abs)}`);
  const sips = Bun.spawnSync([
    "sips", "-Z", String(MAX_PHOTO_PX), "-s", "format", "jpeg",
    "-s", "formatOptions", "62", abs, "--out", tmp,
  ], { stdout: "ignore", stderr: "ignore" });
  if (sips.exitCode === 0) {
    const shrunk = Bun.file(tmp);
    if (await shrunk.exists()) bytes = await shrunk.bytes();
  }
  return {
    uri: `data:image/jpeg;base64,${Buffer.from(bytes).toString("base64")}`,
    bytes: bytes.length,
  };
}

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const SHORT: Record<string, string> = {
  bed: "Bett", desk: "Schreibtisch", wardrobe: "Schrank", couch: "Couch",
  table: "Tisch", coffee_table: "Couchtisch", shelf: "Regal",
};

function label(item: FurnitureItem | undefined, ref: string): string {
  const kind = kindOf({ ref, x: 0, y: 0, rot: 0 });
  if (SHORT[kind]) return SHORT[kind];
  return item?.label?.split(/[,(]/)[0]!.trim() ?? ref;
}

/** The room, its openings, its fixed furniture and one layout, in model coordinates. */
function planSvg(
  room: Room,
  catalogue: Map<string, FurnitureItem>,
  layout: Layout | null,
  cameras: Camera[] = [],
): string {
  const p: string[] = [];
  p.push(`<polygon points="${room.polygon.map((q) => q.join(",")).join(" ")}" class="floor"/>`);

  if (room.bad) {
    const [x0, x1] = room.bad.x;
    const [y0, y1] = room.bad.y;
    p.push(`<rect x="${x0}" y="${y0}" width="${x1 - x0}" height="${y1 - y0}" class="bath"/>`);
    p.push(`<text x="${(x0 + x1) / 2}" y="${(y0 + y1) / 2}" class="zone">BATH</text>`);
  }

  for (const f of room.fixMoebel) {
    const [x0, x1] = f.x;
    const [y0, y1] = f.y;
    p.push(`<rect x="${x0}" y="${y0}" width="${x1 - x0}" height="${y1 - y0}" class="fixed"/>`);
    p.push(`<text x="${(x0 + x1) / 2}" y="${(y0 + y1) / 2}" class="zone">KITCHEN</text>`);
  }

  for (const o of room.oeffnungen) {
    const cls = o.typ === "tuer" ? "door" : "glass";
    const w = room.waende?.[o.wand];
    if (!w) continue;
    const [ax, ay] = w.von;
    const [bx, by] = w.bis;
    const horizontal = ay === by;
    if (horizontal) p.push(`<line x1="${o.von}" y1="${ay}" x2="${o.bis}" y2="${ay}" class="${cls}"/>`);
    else p.push(`<line x1="${ax}" y1="${o.von}" x2="${ax}" y2="${o.bis}" class="${cls}"/>`);
  }

  for (const it of layout?.items ?? []) {
    const cat = catalogue.get(it.ref);
    let fp;
    try { fp = footprint(it, catalogue); } catch { continue; }
    const cx = it.x + fp.w / 2;
    const cy = it.y + fp.d / 2;
    const text = label(cat, it.ref);
    const turn = text.length * 12.6 > fp.w && fp.d > fp.w;
    p.push(`<rect x="${it.x}" y="${it.y}" width="${fp.w}" height="${fp.d}" class="item"/>`);
    p.push(
      `<text x="${cx}" y="${cy}" class="lbl"${turn ? ` transform="rotate(-90 ${cx} ${cy})"` : ""}>${esc(text)}</text>`,
    );
  }

  cameras.forEach((c, i) => {
    const [x, y] = c.pos;
    // A 50 degree cone pointing along `blick`, drawn 90 cm long. Purely indicative.
    const rad = ((c.blick - 90) * Math.PI) / 180;
    const spread = (25 * Math.PI) / 180;
    const len = 95;
    const ax = x + Math.cos(rad - spread) * len;
    const ay = y + Math.sin(rad - spread) * len;
    const bx = x + Math.cos(rad + spread) * len;
    const by = y + Math.sin(rad + spread) * len;
    p.push(`<a href="#foto-${i}" aria-label="${esc(c.titel)}">`);
    p.push(`<polygon points="${x},${y} ${ax},${ay} ${bx},${by}" class="cone"/>`);
    p.push(`<circle cx="${x}" cy="${y}" r="17" class="cam"/>`);
    p.push(`<text x="${x}" y="${y}" class="camnum">${i + 1}</text>`);
    p.push(`</a>`);
  });

  const [minX, minY, w, h] = viewBox(room, cameras);
  return `<svg viewBox="${minX} ${minY} ${w} ${h}" xmlns="http://www.w3.org/2000/svg">${p.join("")}</svg>`;
}

/** Bounds of everything worth showing, including any camera standing outside the walls. */
function viewBox(room: Room, cameras: Camera[] = []): [number, number, number, number] {
  const xs = room.polygon.map((q) => q[0]!);
  const ys = room.polygon.map((q) => q[1]!);
  if (room.bad) { xs.push(...room.bad.x); ys.push(...room.bad.y); }
  for (const c of cameras) { xs.push(c.pos[0]); ys.push(c.pos[1]); }
  const pad = 40;
  const minX = Math.min(...xs) - pad;
  const minY = Math.min(...ys) - pad;
  return [minX, minY, Math.max(...xs) - minX + pad, Math.max(...ys) - minY + pad];
}

const CSS = `
:root{
  --paper:#FBFAF8;--card:#fff;--ink:#16181A;--soft:#5B5F63;--line:#C9C6C0;--hair:#E4E1DB;
  --oak:#B0764A;--oak-ink:#8A5A38;--flag:#B4402F;
  --floor:#F2EFE9;--bathfill:#EAE6DE;--item:#DFD8CC;--itemline:#8C8378;
}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){
  --paper:#131416;--card:#1A1C1F;--ink:#E9E7E3;--soft:#A0A4A8;--line:#3C4045;--hair:#2A2D31;
  --oak:#D89A66;--oak-ink:#E0AC7F;--flag:#E0705C;
  --floor:#212429;--bathfill:#1C1F23;--item:#31353B;--itemline:#8E9298;
}}
:root[data-theme="dark"]{
  --paper:#131416;--card:#1A1C1F;--ink:#E9E7E3;--soft:#A0A4A8;--line:#3C4045;--hair:#2A2D31;
  --oak:#D89A66;--oak-ink:#E0AC7F;--flag:#E0705C;
  --floor:#212429;--bathfill:#1C1F23;--item:#31353B;--itemline:#8E9298;
}
*{box-sizing:border-box}
body{background:var(--paper);color:var(--ink);margin:0;line-height:1.6;
  font-family:ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}
.mono,.eyebrow,.routes,td.num,th,svg text{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.wrap{max-width:1120px;margin:0 auto;padding:clamp(24px,5vw,60px) clamp(16px,4vw,36px) 90px}
h1,h2,h3{margin:0;text-wrap:balance;letter-spacing:-.015em}
h1{font-size:clamp(1.9rem,5vw,2.9rem);line-height:1.05}
h2{font-size:clamp(1.2rem,2.6vw,1.55rem)}
h3{font-size:.98rem}
p{margin:0;max-width:68ch}
.eyebrow{font-size:.7rem;letter-spacing:.14em;text-transform:uppercase;color:var(--soft)}
header{display:flex;flex-direction:column;gap:12px;padding-bottom:30px;border-bottom:1px solid var(--line)}
section{display:flex;flex-direction:column;gap:18px;padding-top:46px}
.plans{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:18px}
.plan{background:var(--card);border:1px solid var(--hair);border-radius:3px;padding:15px;display:flex;flex-direction:column;gap:11px}
.plan.fail{opacity:.62}
.plan svg,.wide svg{width:100%;height:auto;display:block}
.plan-head{display:flex;align-items:baseline;justify-content:space-between;gap:8px}
.tag{font-size:.64rem;font-weight:600;letter-spacing:.09em;text-transform:uppercase;padding:2px 7px;border:1px solid currentColor;border-radius:2px}
.tag.pass{color:var(--oak-ink)}.tag.fail{color:var(--flag)}
.routes{font-size:.73rem;color:var(--soft);border-top:1px solid var(--hair);padding-top:9px;display:flex;flex-direction:column;gap:3px}
.routes b{color:var(--ink)}
.why{font-size:.8rem;color:var(--flag);margin:0}
.wide{background:var(--card);border:1px solid var(--hair);border-radius:3px;padding:18px;max-width:760px}
svg .floor{fill:var(--floor);stroke:var(--ink);stroke-width:5;stroke-linejoin:miter}
svg .bath{fill:var(--bathfill);stroke:var(--line);stroke-width:3}
svg .fixed{fill:none;stroke:var(--itemline);stroke-width:3;stroke-dasharray:12 8}
svg .item{fill:var(--item);stroke:var(--itemline);stroke-width:3}
svg .glass{stroke:var(--oak);stroke-width:9}
svg .door{stroke:var(--soft);stroke-width:9;stroke-dasharray:16 10}
svg text{text-anchor:middle;dominant-baseline:middle;fill:var(--ink)}
svg .lbl{font-size:21px}
svg .zone{font-size:19px;fill:var(--soft)}
svg .cone{fill:var(--oak);opacity:.22}
svg .cam{fill:var(--oak);stroke:var(--paper);stroke-width:3}
svg .camnum{font-size:19px;font-weight:700;fill:var(--paper)}
svg a{cursor:pointer}
svg a:hover .cone{opacity:.42}
svg a:focus-visible .cam{stroke:var(--ink)}
.shots{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px;margin-top:6px}
.shot{display:flex;gap:8px;align-items:baseline;font-size:.8rem;color:var(--soft);text-decoration:none;padding:7px 9px;border:1px solid var(--hair);border-radius:3px;background:var(--card)}
.shot:hover{border-color:var(--oak)}
.shot b{color:var(--oak-ink);font-variant-numeric:tabular-nums}
.lightbox{position:fixed;inset:0;background:rgba(8,9,10,.93);display:none;z-index:50;
  padding:20px;overflow:auto;text-align:center}
.lightbox:target{display:block}
.lightbox img{max-width:100%;max-height:82vh;border-radius:3px}
.lightbox .cap{color:#EDEBE7;font-size:.85rem;margin-top:12px}
.lightbox .close{position:fixed;top:14px;right:18px;color:#EDEBE7;font-size:1.6rem;text-decoration:none;line-height:1}
.scroll{overflow-x:auto}
table{border-collapse:collapse;width:100%;min-width:480px;font-size:.88rem}
th,td{text-align:left;padding:8px 14px 8px 0;border-bottom:1px solid var(--hair);vertical-align:top}
th{font-size:.66rem;letter-spacing:.1em;text-transform:uppercase;color:var(--soft);border-bottom-color:var(--line)}
td.num{text-align:right;padding-right:18px;font-variant-numeric:tabular-nums}
ul{margin:0;padding-left:1.1em;display:flex;flex-direction:column;gap:6px}
.warn{border-left:2px solid var(--flag);padding-left:16px;display:flex;flex-direction:column;gap:8px}
footer{margin-top:70px;padding-top:20px;border-top:1px solid var(--line);color:var(--soft);font-size:.79rem;display:flex;flex-direction:column;gap:6px}
a{color:var(--oak-ink)}
`;

export type PageInput = { title?: string; generatedAt: string };

export async function renderPage(opts: PageInput): Promise<{ html: string; photos: number; bytes: number }> {
  const room = await loadRoom();
  const catalogue = await loadFurniture();
  const raw = Bun.YAML.parse(await Bun.file(join(MODEL_DIR, "room.yaml")).text()) as any;
  const cameras: Camera[] = raw.kameras ?? [];

  const names = (await readdir(LAYOUTS_DIR))
    .filter((f) => f.endsWith(".yaml"))
    .map((f) => basename(f, ".yaml"))
    .sort();

  const judged: Array<{ name: string; layout: Layout; result: CheckResult }> = [];
  for (const n of names) {
    const layout = await loadLayout(n);
    judged.push({ name: n, layout, result: await checkLayout(layout) });
  }
  const passing = judged.filter((j) => j.result.pass);
  const failing = judged.filter((j) => !j.result.pass);

  // Photos live beside the model, one level up from model/.
  const mediaRoot = dirname(MODEL_DIR);
  let totalBytes = 0;
  const shots: Array<{ cam: Camera; uri: string | null }> = [];
  for (const cam of cameras) {
    const got = await photoDataUri(join(mediaRoot, cam.datei));
    if (got) totalBytes += got.bytes;
    shots.push({ cam, uri: got?.uri ?? null });
  }

  const planCard = (j: typeof judged[number]) => {
    const r = j.result;
    const routes = r.metrics.corridors
      .filter((c) => c.widthCm != null)
      .map((c) => `<span>${esc(c.to.replace(/(tuer|zeile)$/, ""))} <b>${c.widthCm} cm</b></span>`)
      .join("");
    const worst = [...r.hard, ...r.soft].slice(0, 2)
      .map((v) => `<p class="why">${esc(v.message)}</p>`).join("");
    return `<article class="plan${r.pass ? "" : " fail"}">
      <div class="plan-head"><h3>${esc(j.layout.name)}</h3>
      <span class="tag ${r.pass ? "pass" : "fail"}">${r.pass ? "Pass" : "Fail"}</span></div>
      ${planSvg(room, catalogue, j.layout)}
      ${routes ? `<div class="routes">${routes}</div>` : ""}
      ${worst}
    </article>`;
  };

  const uncertain = [...catalogue.values()].filter((i) => i.unsicher?.length);

  const html = `<title>${esc(opts.title ?? "Grundriss")}</title>
<style>${CSS}</style>
<div class="wrap">
<header>
  <p class="eyebrow">${room.areaM2.toFixed(2)} m² plannable · ${judged.length} layouts · ${passing.length} pass</p>
  <h1>${esc(opts.title ?? "Grundriss")}</h1>
  <p class="eyebrow">generated ${esc(opts.generatedAt)} from model/ — no number on this page was typed by hand</p>
</header>

<section>
  <h2>Street View</h2>
  <p>Every number is a photograph, standing where it was taken. The cone is the direction it faces. Tap one to open it.</p>
  <div class="wide">${planSvg(room, catalogue, null, cameras)}</div>
  <div class="shots">
    ${shots.map(({ cam }, i) => `<a class="shot" href="#foto-${i}"><b>${i + 1}</b> ${esc(cam.titel)}</a>`).join("")}
  </div>
  ${cameras.some((c) => c.geschaetzt)
    ? `<div class="warn"><p><b>Every camera position here is a guess</b>, inferred from what each photo shows rather than measured. Good enough to put the markers on the sheet, and good enough for nothing else.</p></div>`
    : ""}
</section>

<section>
  <h2>Layouts that pass</h2>
  <div class="plans">${passing.map(planCard).join("")}</div>
</section>

${failing.length ? `<section>
  <h2>And the ones that do not</h2>
  <p>Kept on disk, because a recorded failure is what stops the same arrangement being proposed again.</p>
  <div class="plans">${failing.map(planCard).join("")}</div>
</section>` : ""}

${uncertain.length ? `<section>
  <h2>Not measured yet</h2>
  <p>Every number on this page inherits these guesses.</p>
  <div class="scroll"><table>
    <thead><tr><th>Piece</th><th>Estimated</th><th>Note</th></tr></thead>
    <tbody>${uncertain.map((i) => `<tr><td>${esc(i.label ?? i.id)}</td><td class="mono">${esc((i.unsicher ?? []).join(", "))}</td><td>${esc(i.status ?? "")}</td></tr>`).join("")}</tbody>
  </table></div>
</section>` : ""}

${room.todoAufmass.length ? `<section>
  <h2>Measure at handover</h2>
  <ul>${room.todoAufmass.map((t) => `<li>${esc(t)}</li>`).join("")}</ul>
</section>` : ""}

<footer>
  <p>Geometry from <span class="mono">data/wohnung/model/</span>. Verdicts from <span class="mono">interior check</span>.</p>
  <p>This page is entirely self-contained: no external font, no script, no image from anywhere else.</p>
</footer>
</div>
${shots.map(({ cam, uri }, i) => `<div class="lightbox" id="foto-${i}"><a class="close" href="#">×</a>
  ${uri ? `<img src="${uri}" alt="${esc(cam.titel)}">` : `<p class="cap">image not found: ${esc(cam.datei)}</p>`}
  <p class="cap">${i + 1} · ${esc(cam.titel)}${cam.geschaetzt ? " · position is a guess" : ""}</p></div>`).join("")}
`;

  return { html, photos: shots.filter((s) => s.uri).length, bytes: totalBytes };
}
