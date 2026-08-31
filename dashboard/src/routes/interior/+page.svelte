<script lang="ts">
  /**
   * Rooms, and what stands in them.
   *
   * **This page renders and does not compute.** Every verdict, every corridor width and the
   * plan drawing itself arrive finished from `/interior/api/…`. A clearance rule reimplemented
   * here would be a second answer to a question that already has one, and drift between the
   * two is the failure the capability exists to prevent (PRD B27).
   *
   * The one arithmetic below is cents to euros for display.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import {
    axonStatus,
    interior,
    type InteriorItem,
    type InteriorLayoutDetail,
    type InteriorLayoutSummary,
    type InteriorState,
    type InteriorWishlist,
    type InteriorImpact,
    type InteriorPlacedItem,
    type InteriorAllowed,
    type InteriorRobustheit,
    type InteriorSonne,
    type InteriorEinbringung,
    type InteriorDeklaration,
    type InteriorKaufreihenfolge,
    type InteriorSearchReport,
    type InteriorComposed,
    type InteriorModel,
  } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";

  type View = "plans" | "inventory" | "solve" | "buy";

  let view = $state<View>("plans");
  let loading = $state(true);
  let error = $state<string | null>(null);
  let starting = $state(false);

  let layouts = $state<InteriorLayoutSummary[]>([]);
  let selected = $state<string | null>(null);
  let detail = $state<InteriorLayoutDetail | null>(null);
  let detailError = $state<string | null>(null);

  let inventory = $state<{ item: InteriorItem; state: InteriorState | null }[]>([]);
  let wishlist = $state<InteriorWishlist | null>(null);
  /** The flat itself: its name, its outer measurements, its area. Measured, not drawn from. */
  let room = $state<InteriorModel | null>(null);

  /**
   * The three questions a verdict alone cannot answer, fetched per layout and never guessed:
   * by how much does it pass, does the furniture even get in, and when is the sun on it.
   *
   * Loaded beside the layout rather than with it: each is a separate endpoint and a separate
   * cost, and a plan that will not draw until the sun is computed would be a worse plan.
   */
  let robust = $state<InteriorRobustheit | null>(null);
  let sun = $state<InteriorSonne | null>(null);
  let sunError = $state<string | null>(null);
  let moveIn = $state<InteriorEinbringung[]>([]);

  let declarations = $state<InteriorDeklaration[]>([]);
  let order = $state<InteriorKaufreihenfolge | null>(null);

  /** A hypothetical piece, measured against the entrance before anyone buys it. */
  let probeB = $state(120);
  let probeT = $state(60);
  let probeZerlegbar = $state(false);
  let probeAnswer = $state<string | null>(null);

  let solverBusy = $state(false);
  let solverError = $state<string | null>(null);
  let solverElapsed = $state(0);
  let searchResult = $state<InteriorSearchReport | null>(null);
  let composeResult = $state<InteriorComposed[] | null>(null);
  let moveRefs = $state<string[]>([]);
  let composeRefs = $state<string[]>([]);
  let step = $state(20);

  const euro = (cents: number): string =>
    (cents / 100).toLocaleString("de-DE", { style: "currency", currency: "EUR" });

  const size = (i: InteriorItem): string =>
    i.b !== null && i.t !== null ? `${i.b} × ${i.t}${i.h !== null ? ` × ${i.h}` : ""} cm` : "—";

  /** The lower edge: a product's own price, otherwise the bottom of its estimate. */
  const floorPrice = (i: InteriorItem): number | null => i.preis_cent ?? i.kosten_min_cent;

  const owned = $derived(inventory.filter((r) => r.state === "owned"));
  const gone = $derived(inventory.filter((r) => r.state === "gone"));

  // ─── Dragging on the plan ──────────────────────────────────────────────────
  //
  // The capability draws the plan and every piece arrives as a `<g data-ref>` carrying its
  // model coordinates in centimetres. The SVG's viewBox is in centimetres too, so screen → model
  // is one matrix inversion; the page never holds a scale factor of its own, which would be a
  // second version of the geometry.
  //
  // A drag is UI state until it is saved. Nothing is written while you move.

  let dragging = $state(false);
  let dirty = $state(false);
  let planError = $state<string | null>(null);
  let savingPlan = $state(false);
  let saveAs = $state("");

  /** Snapped to the grid the checker itself rasterises on, so a drop cannot land between cells. */
  const SNAP = 5;

  /**
   * The legal region for the piece currently being dragged.
   *
   * Fetched from the capability when a piece is picked up, and again after a rotation, because
   * turning swaps width and depth and therefore changes where the corner may sit. The page only
   * looks positions up in it — the geometry that produced it stays in one place.
   */
  let allowed = $state<InteriorAllowed | null>(null);

  /** Nearest legal position to a free-dragged one. Walls become edges you cannot cross. */
  function snapToAllowed(x: number, y: number): { x: number; y: number } {
    const a = allowed;
    if (!a || a.rows.length === 0) return { x, y };
    let bestRow = a.rows[0];
    for (const r of a.rows) {
      if (Math.abs(r.y - y) < Math.abs(bestRow.y - y)) bestRow = r;
    }
    let bx = bestRow.x[0][0];
    let bd = Infinity;
    for (const [lo, hi] of bestRow.x) {
      const cand = Math.min(Math.max(x, lo), hi);
      const dist = Math.abs(cand - x);
      if (dist < bd) {
        bd = dist;
        bx = cand;
      }
    }
    return { x: bx, y: bestRow.y };
  }

  /**
   * Where a piece is now, as opposed to where the capability drew it.
   *
   * `data-x`/`data-y` are the RENDERED position and are never written to: an SVG `transform` is
   * measured from where the element was drawn, so overwriting them made every drag after the
   * first render in the wrong place — it looked like the piece snapped back. `nx`/`ny` hold the
   * dragged position instead, and the transform is always taken against the rendered one.
   */
  const posOf = (g: SVGGElement) => ({
    x: Number(g.dataset.nx ?? g.dataset.x ?? 0),
    y: Number(g.dataset.ny ?? g.dataset.y ?? 0),
  });

  /** The position the capability drew this piece at. Fixed for as long as the SVG lives. */
  const drawnAt = (g: SVGGElement) => ({
    x: Number(g.dataset.x ?? 0),
    y: Number(g.dataset.y ?? 0),
  });

  /**
   * The current arrangement: position out of the drawing, every other field out of the layout.
   *
   * A drag can change x, y and rot and nothing else, so nothing else is read back off the SVG.
   * Until 2026-08-31 this built each entry from scratch and `size` fell out — the override that
   * says a table is folded OUT. Eight of the flat's layouts carry one, so moving any piece in
   * them quietly resized a different piece back to its catalogue footprint.
   */
  function itemsFromPlan(root: HTMLElement): InteriorPlacedItem[] {
    const known = new Map((detail?.layout.item ?? []).map((it) => [it.ref, it]));
    return [...root.querySelectorAll<SVGGElement>("g[data-ref]")].map((g) => {
      const ref = g.dataset.ref ?? "";
      return {
        ...known.get(ref),
        ref,
        ...posOf(g),
        rot: Number(g.dataset.rot ?? 0),
      };
    });
  }

  /** Attach drag handlers to a freshly rendered plan. Re-runs whenever the SVG is replaced. */
  function draggable(root: HTMLElement) {
    let active: SVGGElement | null = null;
    let startX = 0;
    let startY = 0;
    let originX = 0;
    let originY = 0;

    const toModel = (svg: SVGSVGElement, clientX: number, clientY: number) => {
      const pt = svg.createSVGPoint();
      pt.x = clientX;
      pt.y = clientY;
      const ctm = svg.getScreenCTM();
      if (!ctm) return { x: 0, y: 0 };
      const p = pt.matrixTransform(ctm.inverse());
      return { x: p.x, y: p.y };
    };

    // Hovering names the piece the detail line describes; picking one up pins it. Without the
    // pin the line would empty itself the moment the pointer left the rectangle, which is
    // exactly when someone is reading it.
    const over = (e: PointerEvent) => {
      const g = (e.target as Element).closest<SVGGElement>("g[data-ref]");
      if (g) focusRef = g.dataset.ref ?? null;
    };

    const out = () => {
      if (!dragging) focusRef = picked;
    };

    const down = (e: PointerEvent) => {
      const g = (e.target as Element).closest<SVGGElement>("g[data-ref]");
      if (!g) return;
      const svg = g.ownerSVGElement;
      if (!svg) return;
      active = g;
      picked = g.dataset.ref ?? null;
      focusRef = picked;
      dragging = true;
      allowed = null;
      if (picked && selected) {
        // Fetched, not computed. Until it arrives the drag is free and the drop still snaps,
        // because the answer is applied on release rather than per frame.
        void interior
          .allowedPositions(selected, picked, Number(g.dataset.rot ?? 0))
          .then((a) => (allowed = a))
          .catch(() => (allowed = null));
      }
      const p = toModel(svg, e.clientX, e.clientY);
      startX = p.x;
      startY = p.y;
      const now = posOf(g);
      originX = now.x;
      originY = now.y;
      // A stale delta from the previous drag would be reused by an `up` with no `move`.
      delete g.dataset.dx;
      delete g.dataset.dy;
      g.style.cursor = "grabbing";
      (e.target as Element).setPointerCapture?.(e.pointerId);
      e.preventDefault();
    };

    const move = (e: PointerEvent) => {
      if (!active) return;
      const svg = active.ownerSVGElement;
      if (!svg) return;
      const p = toModel(svg, e.clientX, e.clientY);
      const dx = Math.round((p.x - startX) / SNAP) * SNAP;
      const dy = Math.round((p.y - startY) / SNAP) * SNAP;
      // Against where it was DRAWN, not where it logically is — the two differ after one drag.
      const base = drawnAt(active);
      active.setAttribute(
        "transform",
        `translate(${originX + dx - base.x} ${originY + dy - base.y})`,
      );
      active.dataset.dx = String(dx);
      active.dataset.dy = String(dy);
    };

    const up = () => {
      if (!active) return;
      const dx = Number(active.dataset.dx ?? 0);
      const dy = Number(active.dataset.dy ?? 0);
      const want = { x: originX + dx, y: originY + dy };
      const landed = snapToAllowed(want.x, want.y);
      active.dataset.nx = String(landed.x);
      active.dataset.ny = String(landed.y);
      // Redraw where it actually landed, measured from where it was drawn.
      const base = drawnAt(active);
      active.setAttribute(
        "transform",
        `translate(${landed.x - base.x} ${landed.y - base.y})`,
      );
      if (landed.x !== want.x || landed.y !== want.y) {
        planError = "Walls are hard: snapped to the nearest position that fits.";
      }
      active.style.cursor = "";
      active = null;
      dragging = false;
      if (landed.x !== originX || landed.y !== originY) dirty = true;
    };

    root.addEventListener("pointerdown", down);
    root.addEventListener("pointerover", over);
    root.addEventListener("pointerout", out);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return {
      destroy() {
        root.removeEventListener("pointerdown", down);
        root.removeEventListener("pointerover", over);
        root.removeEventListener("pointerout", out);
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      },
    };
  }

  let planRoot = $state<HTMLElement | null>(null);
  let picked = $state<string | null>(null);
  let focusRef = $state<string | null>(null);

  /**
   * The focused piece's footprint AS DRAWN, read back out of the plan.
   *
   * Not derived from the catalogue: a 90° turn swaps b and t, and `footprint` is what performs
   * that swap. Recomputing it here would be a second copy of it, and a second copy is what this
   * page exists not to have (PRD B27). The capability writes the answer onto the group.
   */
  let focusPlan = $state<{ w: number; d: number; rot: number } | null>(null);

  // The SVG is replaced wholesale on every re-render, so the highlight and the measured
  // footprint are reapplied rather than kept — there is no element to keep them on.
  $effect(() => {
    if (!planRoot) return;
    const want = picked;
    for (const g of planRoot.querySelectorAll<SVGGElement>("g[data-ref]")) {
      g.classList.toggle("picked", g.dataset.ref === want);
    }
    const g = focusRef
      ? planRoot.querySelector<SVGGElement>(`g[data-ref="${CSS.escape(focusRef)}"]`)
      : null;
    focusPlan = g
      ? {
          w: Number(g.dataset.w ?? 0),
          d: Number(g.dataset.d ?? 0),
          rot: Number(g.dataset.rot ?? 0),
        }
      : null;
  });

  function onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
    if (e.key === "r" || e.key === "R") {
      e.preventDefault();
      void rotatePicked();
    }
    if (e.key === "Escape") {
      picked = null;
      focusRef = null;
    }
  }

  /**
   * Turn the selected piece by 90°.
   *
   * Only multiples of 90 exist: `footprint` swaps width and depth near 90°, and
   * `Seite::gedreht` refuses anything else outright rather than round it a second way. The new
   * position keeps the piece's CENTRE, because rotating about a corner is not what anyone means
   * by turning a wardrobe — the capability re-renders, so the swapped footprint is its answer
   * and not ours.
   */
  async function rotatePicked(): Promise<void> {
    if (!planRoot || picked === null || selected === null) return;
    const items = itemsFromPlan(planRoot).map((it) => {
      if (it.ref !== picked) return it;
      const g = planRoot!.querySelector<SVGGElement>(`g[data-ref="${CSS.escape(it.ref)}"]`);
      const w = Number(g?.dataset.w ?? 0);
      const d = Number(g?.dataset.d ?? 0);
      const cx = it.x + w / 2;
      const cy = it.y + d / 2;
      const snap = (v: number) => Math.round(v / SNAP) * SNAP;
      // After the turn the footprint is swapped, so the centre implies a new top-left.
      return { ...it, rot: (it.rot + 90) % 360, x: snap(cx - d / 2), y: snap(cy - w / 2) };
    });
    savingPlan = true;
    planError = null;
    try {
      detail = await interior.previewLayout(selected, items);
      dirty = true;
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  async function savePlan(): Promise<void> {
    if (!planRoot || selected === null) return;
    savingPlan = true;
    planError = null;
    try {
      detail = await interior.saveLayout(selected, itemsFromPlan(planRoot));
      dirty = false;
      layouts = await interior.layouts();
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  async function savePlanAs(): Promise<void> {
    if (!planRoot || !saveAs.trim()) return;
    savingPlan = true;
    planError = null;
    try {
      const id = saveAs.trim();
      const stamp = new Date().toISOString().slice(0, 10);
      await interior.createLayout({
        id,
        items: itemsFromPlan(planRoot),
        notiz: `Am ${stamp} in der Oberflaeche gestellt, ausgehend von ${selected ?? "einem leeren Plan"}.`,
      });
      saveAs = "";
      dirty = false;
      layouts = await interior.layouts();
      await select(id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  // ─── A new plan ────────────────────────────────────────────────────────────

  let planNew = $state(false);
  let newId = $state("");
  /** Start from the layout on screen, or from an empty room. */
  let newCopy = $state(true);

  /**
   * Why the name is refused, or null.
   *
   * The same shape the ids on disk already have, and a file name that needs no escaping: the
   * capability turns this into `layouts/<id>.toml`. Checked here so the answer arrives while
   * typing, and again in the capability, which is the one that owns the file.
   */
  function nameProblem(v: string): string | null {
    const id = v.trim();
    if (id === "") return null;
    if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(id)) {
      return "lowercase letters, digits and single hyphens only";
    }
    if (layouts.some((l) => l.id === id)) return `${id} already exists`;
    return null;
  }
  const newProblem = $derived(nameProblem(newId));

  async function createPlan(): Promise<void> {
    const id = newId.trim();
    if (!id || newProblem) return;
    savingPlan = true;
    planError = null;
    try {
      // `von` copies server-side: the page has no business rebuilding an arrangement it
      // already asked for once. An empty plan sends neither, and the capability writes a
      // header saying the date and that the API made it — not a provenance it does not have.
      await interior.createLayout({ id, von: newCopy && selected ? selected : undefined });
      newId = "";
      planNew = false;
      dirty = false;
      layouts = await interior.layouts();
      await select(id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  /** Second click confirms. Nothing is destroyed — the file moves to `layouts/archiv/`. */
  let archiveArmed = $state<string | null>(null);

  async function archivePlan(): Promise<void> {
    if (selected === null) return;
    if (archiveArmed !== selected) {
      archiveArmed = selected;
      return;
    }
    savingPlan = true;
    planError = null;
    try {
      const r = await interior.archiveLayout(selected);
      planError = `Moved to ${r.archiviert}. Nothing was deleted.`;
      archiveArmed = null;
      selected = null;
      detail = null;
      layouts = await interior.layouts();
      if (layouts.length > 0) await select(layouts[0].id);
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  /** Record the arrangement as where things actually stand, not as a proposal. */
  async function markAsActual(): Promise<void> {
    if (!planRoot) return;
    savingPlan = true;
    planError = null;
    try {
      const r = await interior.savePlacements(itemsFromPlan(planRoot));
      planError = `Recorded ${r.gesetzt} pieces as actually placed.`;
    } catch (caught) {
      planError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  // ─── Editing ───────────────────────────────────────────────────────────────
  //
  // Every write goes through the capability. The page holds no rule and computes no verdict:
  // even the impact preview below is the capability re-checking every layout, not the browser
  // guessing. A second implementation of a clearance rule is the drift this whole thing exists
  // to prevent (PRD B27).

  let editing = $state<string | null>(null);
  /** Form values are strings while they are being typed; `patchFrom` converts on the way out. */
  type Draft = {
    label: string;
    b: string | number;
    t: string | number;
    h: string | number;
    preis_cent: string | number;
    prioritaet: string;
    hinweis: string;
    begruendung: string;
    opens: string;
    open_clear: string | number;
    wall_ok: string;
    access_sides: string | number;
    access_clear: string | number;
    raumtrenner: boolean;
    bild: string;
  };
  let draft = $state<Draft>({
    label: "", b: "", t: "", h: "", preis_cent: "", prioritaet: "", hinweis: "",
    begruendung: "", opens: "", open_clear: "", wall_ok: "", access_sides: "",
    access_clear: "", raumtrenner: false, bild: "",
  });
  let impact = $state<InteriorImpact | null>(null);
  let impactBusy = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let history = $state<{ state: InteriorState; since: string; note: string | null }[]>([]);
  let creating = $state(false);
  let draftNew = $state({ id: "", label: "", kind: "piece" as "piece" | "slot", state: "wanted" as InteriorState, b: "", t: "", h: "", preis: "" });

  const byId = $derived(new Map(inventory.map((r) => [r.item.id, r])));

  /** Which Q61 fields a piece fills in, if any. Empty means the name heuristic still judges it. */
  const declares = (i: InteriorItem): string =>
    [
      i.opens !== null && i.opens !== undefined ? "opens" : null,
      i.open_clear !== null && i.open_clear !== undefined ? "open_clear" : null,
      i.expands_to !== null && i.expands_to !== undefined ? "expands" : null,
      i.access_sides !== null && i.access_sides !== undefined ? "access" : null,
      i.raumtrenner ? "raumtrenner" : null,
    ]
      .filter(Boolean)
      .join(" · ");

  /** cm and € come out of inputs as strings; empty means "clear it", not "zero". */
  const num = (v: unknown): number | null => {
    const t = String(v ?? "").trim();
    if (t === "") return null;
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
  };

  async function openEditor(id: string): Promise<void> {
    const row = byId.get(id);
    if (!row) return;
    editing = id;
    saveError = null;
    impact = null;
    const i = row.item;
    draft = {
      label: i.label,
      b: i.b ?? "",
      t: i.t ?? "",
      h: i.h ?? "",
      preis_cent: i.preis_cent ?? "",
      prioritaet: i.prioritaet ?? "",
      hinweis: i.hinweis ?? "",
      begruendung: i.begruendung ?? "",
      opens: i.opens ?? "",
      open_clear: i.open_clear ?? "",
      wall_ok: i.wall_ok === null ? "" : String(i.wall_ok),
      access_sides: i.access_sides ?? "",
      access_clear: i.access_clear ?? "",
      raumtrenner: i.raumtrenner ?? false,
      bild: i.bild ?? "",
    };
    history = [];
    try {
      history = await interior.stateHistory(id);
    } catch {
      // A missing history is not a reason to refuse the edit; the panel says so itself.
    }
  }

  /** Only the fields that actually differ. Sending the unchanged ones would work, but a diff
      that names exactly what moved is what makes the impact preview readable. */
  function patchFrom(id: string): Record<string, unknown> {
    const i = byId.get(id)?.item;
    if (!i) return {};
    const out: Record<string, unknown> = {};
    const put = (k: string, v: unknown, was: unknown) => {
      if (v !== was) out[k] = v;
    };
    put("label", String(draft.label ?? "").trim(), i.label);
    put("b", num(draft.b), i.b);
    put("t", num(draft.t), i.t);
    put("h", num(draft.h), i.h);
    put("preis_cent", num(draft.preis_cent), i.preis_cent);
    put("prioritaet", String(draft.prioritaet ?? "").trim() || null, i.prioritaet);
    put("hinweis", String(draft.hinweis ?? "").trim() || null, i.hinweis);
    put("begruendung", String(draft.begruendung ?? "").trim() || null, i.begruendung);
    put("opens", String(draft.opens ?? "") || null, i.opens);
    put("open_clear", num(draft.open_clear), i.open_clear);
    put("wall_ok", draft.wall_ok === "" ? null : draft.wall_ok === "true", i.wall_ok);
    put("access_sides", num(draft.access_sides), i.access_sides);
    put("access_clear", num(draft.access_clear), i.access_clear);
    put("raumtrenner", draft.raumtrenner ? true : null, i.raumtrenner);
    put("bild", String(draft.bild ?? "").trim() || null, i.bild);
    return out;
  }

  async function preview(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      impact = null;
      return;
    }
    impactBusy = true;
    saveError = null;
    try {
      impact = await interior.impact(editing, patch);
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      impactBusy = false;
    }
  }

  async function save(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      editing = null;
      return;
    }
    saving = true;
    saveError = null;
    try {
      await interior.patchItem(editing, patch);
      editing = null;
      impact = null;
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function move(id: string, to: InteriorState): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await interior.setState(id, to);
      if (editing === id) history = await interior.stateHistory(id);
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function create(): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await interior.createItem({
        id: draftNew.id.trim(),
        kind: draftNew.kind,
        label: draftNew.label.trim(),
        state: draftNew.state,
        b: num(draftNew.b),
        t: num(draftNew.t),
        h: num(draftNew.h),
        preis_cent: num(draftNew.preis),
      });
      creating = false;
      draftNew = { id: "", label: "", kind: "piece", state: "wanted", b: "", t: "", h: "", preis: "" };
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      const [l, inv, wish, m] = await Promise.all([
        interior.layouts(),
        interior.inventory(),
        interior.wishlist(),
        interior.model(),
      ]);
      layouts = l;
      inventory = inv;
      wishlist = wish;
      room = m;
      if (selected === null && l.length > 0) await select(l[0].id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      loading = false;
    }
  }

  async function select(id: string): Promise<void> {
    selected = id;
    detail = null;
    detailError = null;
    picked = null;
    focusRef = null;
    archiveArmed = null;
    dirty = false;
    robust = null;
    sun = null;
    sunError = null;
    moveIn = [];
    try {
      detail = await interior.layout(id);
    } catch (caught) {
      detailError = caught instanceof Error ? caught.message : String(caught);
      return;
    }
    // Beside the plan, not before it. Each answers its own question and each may fail on its
    // own — a flat without `[lage]` has no sun report and still has a layout.
    void interior
      .toleranz(id)
      .then((r) => (robust = r))
      .catch(() => (robust = null));
    void interior
      .einbringung(id)
      .then((r) => (moveIn = r))
      .catch(() => (moveIn = []));
    void interior
      .sonne(id)
      .then((r) => (sun = r))
      .catch((e) => (sunError = e instanceof Error ? e.message : String(e)));
  }

  /** How much error the verdict survives, in one sentence. */
  const holdsUpTo = (r: InteriorRobustheit): string => {
    switch (r.haelt.art) {
      case "faellt_durch":
        return "fails already at the measurements on file";
      case "nichts_geraten":
        return "no measurement in this layout is marked as a guess";
      case "bis":
        return `holds up to ${r.haelt.cm} cm of measurement error`;
      case "ueber_horizont":
        return `holds past ${r.haelt.horizont_cm} cm — further was not searched`;
    }
  };

  const doorLine = (e: InteriorEinbringung): string => {
    switch (e.tuer.art) {
      case "passt":
        return `through the ${e.tuer.tuer_cm} cm door, ${e.tuer.luft_cm} cm to spare`;
      case "passt_nicht":
        return `does NOT fit the ${e.tuer.tuer_cm} cm door — ${e.tuer.fehlen_cm} cm too wide`;
      case "zerlegt_getragen":
        return `${e.tuer.fehlen_cm} cm wider than the ${e.tuer.tuer_cm} cm door, carried in parts`;
      case "kein_eingang":
        return "no opening is declared as the entrance";
    }
  };

  async function loadDeclarations(): Promise<void> {
    try {
      declarations = await interior.deklaration();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function loadOrder(): Promise<void> {
    try {
      order = await interior.kaufen();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function probeDoor(): Promise<void> {
    probeAnswer = null;
    try {
      const t = await interior.passt(probeB, probeT, probeZerlegbar);
      probeAnswer = doorLine({
        reference: "",
        b: probeB,
        t: probeT,
        tuer: t,
        erreichbar: false,
        dreht: false,
      });
    } catch (caught) {
      probeAnswer = caught instanceof Error ? caught.message : String(caught);
    }
  }

  /**
   * Wait for a job, then hand back its result.
   *
   * Polling and not a socket: the answer arrives once, minutes from now, and a second
   * transport for one message would be more machinery than the message is worth. The interval
   * is a second because the job takes minutes — a tighter loop would ask the same question
   * sixty times for one answer.
   */
  async function awaitJob<T>(id: number): Promise<T> {
    const started = Date.now();
    for (;;) {
      const { stand } = await interior.auftrag(id);
      if (stand.zustand === "fertig") return stand.ergebnis as T;
      if (stand.zustand === "gescheitert") throw new Error(stand.grund);
      solverElapsed = Math.round((Date.now() - started) / 1000);
      await new Promise((r) => setTimeout(r, 1000));
    }
  }

  async function runSearch(): Promise<void> {
    if (selected === null || moveRefs.length === 0) return;
    solverBusy = true;
    solverError = null;
    searchResult = null;
    composeResult = null;
    try {
      const { auftrag } = await interior.search(selected, moveRefs, step, 12);
      searchResult = await awaitJob<InteriorSearchReport>(auftrag);
    } catch (caught) {
      solverError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      solverBusy = false;
    }
  }

  async function runCompose(): Promise<void> {
    if (composeRefs.length === 0) return;
    solverBusy = true;
    solverError = null;
    searchResult = null;
    composeResult = null;
    try {
      const { auftrag } = await interior.compose(composeRefs, 25, 60, 5);
      composeResult = await awaitJob<InteriorComposed[]>(auftrag);
    } catch (caught) {
      solverError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      solverBusy = false;
    }
  }

  function toggle(list: string[], id: string): string[] {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }

  async function start(): Promise<void> {
    starting = true;
    error = null;
    try {
      await axonStatus.start("interior");
      await capabilities.refresh();
      await load();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      starting = false;
    }
  }

  onMount(() => {
    capabilities.subscribe();
    void load();
  });
</script>

<svelte:window onkeydown={onKey} />

<PageHeader
  badge="Interior"
  title="Rooms and what stands in them"
  desc="Layouts judged against the flat's own clearance rules, and an inventory that outlives the flat."
/>

{#if error}
  <div class="card offer">
    <p class="lead"><Icon name="alert" size={15} /> {error}</p>
    <!-- On-demand is the design: nothing but the shell runs until you open something. -->
    <button onclick={start} disabled={starting}>
      {starting ? "Starting…" : "Start interior"}
    </button>
  </div>
{:else if loading}
  <p class="empty">Reading the model…</p>
{:else}
  <nav class="views" aria-label="Interior views">
    <button class:active={view === "plans"} onclick={() => (view = "plans")}>
      Plans <span class="count">{layouts.length}</span>
    </button>
    <button class:active={view === "inventory"} onclick={() => (view = "inventory")}>
      Inventory <span class="count">{inventory.length}</span>
    </button>
    <button
      class:active={view === "solve"}
      onclick={() => (view = "solve")}
    >
      Solve
    </button>
    <button
      class:active={view === "buy"}
      onclick={() => {
        view = "buy";
        if (order === null) void loadOrder();
        if (declarations.length === 0) void loadDeclarations();
      }}
    >
      Buy &amp; declare
    </button>
  </nav>
{/if}

{#if !loading && !error && view === "plans"}
  {#if layouts.length === 0}
    <p class="empty">No layout on disk for this flat.</p>
  {:else}
    <div class="plans">
      <!-- One grid column: the list of plans, and the way to add one. -->
      <div class="side">
        <ul class="variants">
          {#each layouts as l (l.id)}
            <li>
              <button class:active={selected === l.id} onclick={() => select(l.id)}>
                <span class="verdict" class:pass={l.pass}>{l.pass ? "passes" : "fails"}</span>
                <span class="name">{l.name}</span>
                <span class="tally mono">
                  {l.hard} hard · {l.soft} soft
                </span>
              </button>
            </li>
          {/each}
        </ul>

        <!--
          A plan of one's own. The name becomes `layouts/<id>.toml`, so it is checked against
          the shape the ids on disk already have — and against the ones that exist, because the
          capability refuses to overwrite and this says so before the round trip.
        -->
        <div class="newplan">
          <button class="ghost" onclick={() => (planNew = !planNew)}>
            {planNew ? "Cancel" : "+ New plan"}
          </button>
          {#if planNew}
            <input placeholder="bett-an-der-ostwand" bind:value={newId} />
            <label>
              <input type="checkbox" bind:checked={newCopy} disabled={selected === null} />
              start from {selected ?? "an empty room"}
            </label>
            {#if newProblem}<p class="bad">{newProblem}</p>{/if}
            <button
              disabled={newId.trim() === "" || newProblem !== null || savingPlan}
              onclick={createPlan}
            >
              {savingPlan ? "Creating…" : "Create"}
            </button>
          {/if}
        </div>
      </div>

      <section class="plan">
        {#if detailError}
          <p class="error"><Icon name="alert" size={15} /> {detailError}</p>
        {:else if detail === null}
          <p class="empty">Drawing…</p>
        {:else}
          <!--
            Which plan, and how big the room it is drawn in actually is. The measurements come
            from the capability, which reads them off the same polygon every check rasterises.
          -->
          <p class="where">
            <strong>{detail.layout.name}</strong>
            <span class="mono id">{selected}</span>
            {#if room}
              <span class="of">
                {room.flat.name} · {room.masse.b} × {room.masse.t} cm ·
                {room.area_m2.toFixed(1)} m²
              </span>
            {/if}
          </p>
          <!-- The capability draws it. eslint-disable-next-line svelte/no-at-html-tags -->
          <!--
            Drag a piece; nothing is written until you save. The verdict below is the
            capability's, recomputed on the arrangement you land on — the page decides nothing.
          -->
          {#key detail.svg}
            <div class="svg" class:dragging bind:this={planRoot} use:draggable>{@html detail.svg}</div>
          {/key}

          <!--
            What the piece under the pointer measures. The footprint is the one the capability
            DREW: a 90° turn swaps b and t, and `footprint` is what swaps them. Everything below
            it is the catalogue row, including the clearances the piece states for itself (Q61)
            — a wardrobe that needs 65 cm in front of its doors says so here, not in a rule.
          -->
          {#if focusRef}
            {@const it = byId.get(focusRef)?.item}
            <p class="piece">
              <span class="mono ref">{focusRef}</span>
              {#if it}<span class="what">{it.label}</span>{/if}
              {#if focusPlan}
                <strong class="mono">{focusPlan.w} × {focusPlan.d} cm</strong>
                <span class="of">
                  in the plan{focusPlan.rot ? `, turned ${focusPlan.rot}°` : ""}
                </span>
              {/if}
              {#if it}
                <span class="of mono">
                  h {it.h ?? "—"}{it.h_min !== null ? ` · h_min ${it.h_min}` : ""} cm
                </span>
                {#if it.b !== null && it.t !== null}
                  <span class="of mono">catalogue {it.b} × {it.t}</span>
                {/if}
                {#if it.open_clear !== null}
                  <span class="needs mono">
                    open_clear {it.open_clear} cm{it.opens ? ` ${it.opens}` : ""}
                  </span>
                {/if}
                {#if it.access_sides !== null}
                  <span class="needs mono">
                    access_sides {it.access_sides}{it.access_clear !== null
                      ? ` × ${it.access_clear} cm`
                      : ""}
                  </span>
                {/if}
                {#if it.expands_to !== null}
                  <span class="needs mono">
                    expands {it.expands_dir ?? "?"} to {it.expands_to} cm
                  </span>
                {/if}
                {#if it.raumtrenner}<span class="needs mono">raumtrenner</span>{/if}
                {#if it.unsicher.length > 0}
                  <span class="guess" title="measured? no.">~ {it.unsicher.join(", ")}</span>
                {/if}
              {:else}
                <span class="of">no inventory row — the plan draws it from its own size</span>
              {/if}
            </p>
          {/if}
          <div class="planbar">
            <button class="ghost" disabled={picked === null || savingPlan} onclick={rotatePicked}>
              {picked === null ? "Rotate (pick a piece)" : `Rotate ${picked} 90°`}
            </button>
            {#if dirty}<span class="moved">moved — not saved</span>{/if}
            <button class="ghost" disabled={!dirty || savingPlan} onclick={savePlan}>
              {savingPlan ? "Saving…" : "Save into this layout"}
            </button>
            <input placeholder="save as new id…" bind:value={saveAs} />
            <button class="ghost" disabled={!saveAs.trim() || savingPlan} onclick={savePlanAs}>
              Save as
            </button>
            <button class="ghost" disabled={savingPlan} onclick={markAsActual}>
              This is how it stands
            </button>
            <!-- Not a delete. The file moves to layouts/archiv/ and keeps its reasoning. -->
            <button
              class="ghost"
              class:armed={archiveArmed === selected}
              disabled={savingPlan}
              onclick={archivePlan}
            >
              {archiveArmed === selected ? "Archive — click again" : "Archive"}
            </button>
          </div>
          {#if planError}<p class="note">{planError}</p>{/if}

          <!--
            By how much, not whether. Two layouts that both report `pass` can have 2 cm and
            40 cm of air, and until 2026-08-31 they were the same word in every report.
          -->
          {#if detail.check.engste_reserve_cm !== null}
            <p class="reserve" class:tight={detail.check.engste_reserve_cm < 10}>
              <strong class="mono">{detail.check.engste_reserve_cm} cm</strong>
              {detail.check.engste_reserve_cm < 0
                ? "short at its tightest hard rule"
                : "is all the room the tightest hard rule has left"}
              {#if robust}<span class="holds">· {holdsUpTo(robust)}</span>{/if}
              {#if robust && robust.kippt_an.length > 0}
                <span class="holds">· breaks first at {robust.kippt_an.join(", ")}</span>
              {/if}
            </p>
          {/if}

          <dl class="metrics mono">
            <div><dt>room</dt><dd>{detail.check.metrics.room_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>occupied</dt><dd>{detail.check.metrics.occupied_area_m2.toFixed(2)} m²</dd></div>
            <div><dt>free</dt><dd>{detail.check.metrics.free_area_m2.toFixed(2)} m²</dd></div>
          </dl>

          <table class="corridors">
            <caption>Walks, at their narrowest point</caption>
            <tbody>
              {#each detail.check.metrics.corridors as c (c.from + c.to)}
                <tr>
                  <th scope="row">{c.from} → {c.to}</th>
                  <td class="mono">{c.width_cm === null ? "no way through" : `${c.width_cm} cm`}</td>
                </tr>
              {/each}
            </tbody>
          </table>

          {#if detail.check.hard.length > 0}
            <ul class="violations hard">
              {#each detail.check.hard as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if detail.check.soft.length > 0}
            <ul class="violations soft">
              {#each detail.check.soft as v, i (v.rule + i)}
                <li>
                  <span class="rule mono">{v.rule}</span>
                  {v.message}
                  {#if v.text}<span class="rule-text">{v.text}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          <!--
            Declared and not measured. Shown on a passing layout too, and that is the point:
            "passes" means "passes the rules that were checked", and this is the difference
            between the two. It was invisible until 2026-08-31.
          -->
          {#if detail.check.nicht_geprueft.length > 0}
            <p class="unchecked">
              <Icon name="alert" size={14} />
              Not measured here:
              {detail.check.nicht_geprueft.map((r) => `${r.rule} (${r.grund})`).join(" · ")}
            </p>
          {/if}
          <!--
            Furniture already decided against. A verdict about pieces that are not coming
            answers no question that is still open — and one of the failing layouts fails on
            exactly such a piece.
          -->
          {#if detail.check.veraltet.length > 0}
            <p class="unchecked">
              <Icon name="alert" size={14} />
              Places furniture already rejected: {detail.check.veraltet.join(", ")} — this
              verdict is about a plan you have moved on from.
            </p>
          {/if}
          {#if detail.check.uncertainties.length > 0}
            <p class="guessed">
              <Icon name="alert" size={14} />
              Every number above inherits a guess:
              {detail.check.uncertainties.map((u) => `${u.label} (${u.fields.join(", ")})`).join("; ")}
            </p>
          {/if}

          <!--
            Every measurement, including the ones that passed. The table is the difference
            between "this layout works" and "this layout works, and here is where it is thin".
          -->
          {#if detail.check.reserven.length > 0}
            <details class="reserves">
              <summary>Every measurement, passed or not ({detail.check.reserven.length})</summary>
              <table class="corridors">
                <tbody>
                  {#each detail.check.reserven as r, i (r.rule + i)}
                    <tr class:tight={r.hart && r.bindend && r.slack < 10}>
                      <th scope="row">
                        {r.rule}{#if r.item}<span class="of"> · {r.item}</span>{/if}{#if r.bezug}<span class="of"> · {r.bezug}</span>{/if}
                      </th>
                      <td class="mono">
                        {r.measured}/{r.required} {r.einheit}
                      </td>
                      <td class="mono slack" class:neg={r.slack < 0}>
                        {r.gedeckelt ? "≥ " : ""}{r.slack > 0 ? "+" : ""}{r.slack}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </details>
          {/if}

          <!--
            A layout can satisfy every clearance rule and still never happen, because the piece
            does not fit through the front door. Until 2026-08-31 the answer to that was
            "passes" — computed right, wrong question.
          -->
          {#if moveIn.length > 0}
            <details class="reserves" open={moveIn.some((e) => e.tuer.art === "passt_nicht" || !e.erreichbar)}>
              <summary>Getting it in ({moveIn.length})</summary>
              <ul class="movein">
                {#each moveIn as e (e.reference)}
                  <li class:bad={e.tuer.art === "passt_nicht" || !e.erreichbar}>
                    <span class="mono">{e.reference}</span>
                    <span class="dims mono">{e.b}×{e.t}</span>
                    <span>{doorLine(e)}</span>
                    <span class="way">
                      {e.erreichbar
                        ? `route free${e.dreht ? ", with a turn" : ""}`
                        : (e.grund ?? "no way to its place")}
                    </span>
                  </li>
                {/each}
              </ul>
            </details>
          {/if}

          <!--
            Four days carry the whole year: the two solstices are the extremes, the equinoxes
            the middle. Every other day lies between them.
          -->
          {#if sun}
            <details class="reserves">
              <summary>Sun over the year</summary>
              <table class="sun mono">
                <tbody>
                  {#each [...new Set(sun.stunden.map((s) => s.tag))] as tag (tag)}
                    <tr>
                      <th scope="row">{tag}</th>
                      {#each sun.stunden.filter((s) => s.tag === tag) as h (h.stunde_lokal)}
                        <td
                          class:night={h.hoehe_grad <= 0}
                          class:hit={h.getroffen.length > 0}
                          title={`${h.stunde_lokal}:00 · ${h.hoehe_grad.toFixed(0)}° up, ${h.azimut_grad.toFixed(0)}° · ${h.getroffen.join(", ") || "nothing in the sun"}`}
                        >
                          {h.hoehe_grad <= 0 ? "·" : h.getroffen.length || "–"}
                        </td>
                      {/each}
                    </tr>
                  {/each}
                </tbody>
              </table>
              <ul class="sunlist">
                {#each Object.entries(sun.treffer_je_stueck) as [ref, n] (ref)}
                  <li><span class="mono">{ref}</span> {n} hours in direct sun</li>
                {/each}
              </ul>
              {#if sun.ohne_glashoehen.length > 0}
                <p class="note">
                  No glazing heights on file for {sun.ohne_glashoehen.join(", ")} — those
                  windows throw no light here, and they should.
                </p>
              {/if}
            </details>
          {:else if sunError}
            <p class="note">No sun report: {sunError}</p>
          {/if}
        {/if}
      </section>
    </div>
  {/if}
{/if}

{#if !loading && !error && view === "solve"}
  <!--
    The most expensive calculation in the capability, finally reachable from the surface that
    needs it. It runs as a job because it takes minutes: the page asks for a number and comes
    back for the answer (PRD B36).
  -->
  <section class="card solver">
    <h2>Move pieces in {selected ?? "a layout"}</h2>
    {#if selected === null}
      <p class="empty">Pick a layout under Plans first.</p>
    {:else}
      <p class="note">
        Every position on the grid is checked, not the first that works. A ★ marks the Pareto
        front: under no weighting of the four goals is such a hit worse than another.
      </p>
      <div class="picks">
        {#each detail?.layout.item ?? [] as it (JSON.stringify(it))}
          {@const ref = (it as { ref: string }).ref}
          <label>
            <input
              type="checkbox"
              checked={moveRefs.includes(ref)}
              onchange={() => (moveRefs = toggle(moveRefs, ref))}
            />
            {ref}
          </label>
        {/each}
      </div>
      <label class="stepper">
        grid <input type="number" min="5" max="50" step="5" bind:value={step} /> cm
      </label>
      <button disabled={solverBusy || moveRefs.length === 0} onclick={runSearch}>
        {solverBusy ? `Searching… ${solverElapsed}s` : "Search"}
      </button>
    {/if}
  </section>

  <section class="card solver">
    <h2>Lay out the flat from nothing</h2>
    <p class="note">
      A beam search, and it says so: only the best partial arrangements survive each round, so
      the answer is computed and reproducible — not proven optimal.
    </p>
    <div class="picks">
      {#each owned as row (row.item.id)}
        <label>
          <input
            type="checkbox"
            checked={composeRefs.includes(row.item.id)}
            onchange={() => (composeRefs = toggle(composeRefs, row.item.id))}
          />
          {row.item.label}
        </label>
      {/each}
    </div>
    <button disabled={solverBusy || composeRefs.length === 0} onclick={runCompose}>
      {solverBusy ? `Placing… ${solverElapsed}s` : "Lay it out"}
    </button>
  </section>

  {#if solverError}<p class="error"><Icon name="alert" size={15} /> {solverError}</p>{/if}

  {#if searchResult}
    <section class="card">
      <h2>
        {searchResult.hits.length} of {searchResult.fully_checked.toLocaleString()} checked
        <span class="note">in {(searchResult.elapsed_ms / 1000).toFixed(1)} s</span>
      </h2>
      <table class="corridors">
        <tbody>
          {#each searchResult.hits as h, i (i)}
            <tr>
              <th scope="row">{h.pareto ? "★" : ""} {Object.entries(h.places).map(([k, p]) => `${k} ${p[0]},${p[1]}`).join(" · ")}</th>
              <td class="mono">{h.engste_reserve_cm ?? "—"} cm spare</td>
              <td class="mono">{h.wandkontakt_cm} cm wall</td>
              <td class="mono">{h.bottleneck_cm} cm walk</td>
              <td class="mono">{h.soft} soft</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  {#if composeResult}
    <section class="card">
      <h2>{composeResult.length} arrangements</h2>
      {#each composeResult as c, i (i)}
        <article class="composed">
          <h3>
            #{i + 1} {c.pareto ? "★" : ""}
            <span class="verdict" class:pass={c.pass}>{c.pass ? "passes" : "fails"}</span>
          </h3>
          <p class="mono">
            {c.engste_reserve_cm ?? "—"} cm spare · {c.wandkontakt_cm} cm wall ·
            {c.bottleneck_cm} cm walk · {c.free_m2.toFixed(2)} m² free
          </p>
          {#if c.hard.length > 0}<p class="note">hard: {c.hard.join(", ")}</p>{/if}
          <ul class="places mono">
            {#each c.places as [ref, p, rot] (ref)}
              <li>{ref} — x {p[0]}, y {p[1]}, {rot}°</li>
            {/each}
          </ul>
        </article>
      {/each}
    </section>
  {/if}
{/if}

{#if !loading && !error && view === "buy"}
  <section class="card">
    <h2>Does it even fit through the door?</h2>
    <p class="note">The question before buying, answered from the flat's own entrance width.</p>
    <div class="probe">
      <label>width <input type="number" bind:value={probeB} /></label>
      <label>depth <input type="number" bind:value={probeT} /></label>
      <label>
        <input type="checkbox" bind:checked={probeZerlegbar} /> comes in parts
      </label>
      <button onclick={probeDoor}>Check</button>
    </div>
    {#if probeAnswer}<p class="reserve">{probeAnswer}</p>{/if}
  </section>

  {#if order}
    <section class="card">
      <h2>What to buy first</h2>
      <p class="note">
        Urgency, then whether a layout already builds on it, then price. No budget: nobody set a
        ceiling and this page does not invent one.
      </p>
      <table class="corridors">
        <tbody>
          {#each order.posten as p, i (p.id)}
            <tr>
              <th scope="row">{i + 1}. {p.label}</th>
              <td class="mono">{p.prioritaet ?? "—"}</td>
              <td class="mono">{euro(p.preis_cent ?? 0)}</td>
              <td class="mono">{euro(p.kumuliert_cent)}</td>
              <!--
                Two reasons for no month count, and they must not read the same: "no balance"
                is a missing measurement, "the balance is not positive" is an answer.
              -->
              <td class="mono">
                {p.erreichbar_nach_monaten !== null
                  ? `${p.erreichbar_nach_monaten.toFixed(1)} months`
                  : order.saldo
                    ? "not out of this balance"
                    : "no balance measured"}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if order.ohne_preis.length > 0}
        <p class="unchecked">
          <Icon name="alert" size={14} />
          {order.ohne_preis.length} needs carry no price and stand in no order:
          {order.ohne_preis.map((p) => p.label).join(", ")} — the move here is a price, not a
          purchase.
        </p>
      {/if}
      {#if order.unbekannte_prioritaeten.length > 0}
        <p class="unchecked">
          <Icon name="alert" size={14} />
          Priority words this ranking does not know, sorted last:
          {order.unbekannte_prioritaeten.join(", ")}
        </p>
      {/if}
      {#if order.saldo && order.saldo.median_cent <= 0}
        <p class="note">
          Median monthly balance {euro(order.saldo.median_cent)} over {order.saldo.monate}
          months — nothing accrues out of that, so there is no month count.
        </p>
      {/if}
    </section>
  {/if}

  {#if declarations.length > 0}
    <section class="card">
      <h2>
        Still judged by name
        <span class="count">{declarations.filter((d) => !d.erklaert).length}</span>
      </h2>
      <p class="note">
        The engine has been cleverer than its data since 2026-08-31. Each line below is what the
        machine already assumes, written in the vocabulary the piece can carry itself — and what
        accepting it would change.
      </p>
      <ul class="declarations">
        {#each declarations.filter((d) => !d.erklaert && d.vorschlag) as d (d.id)}
          <li>
            <span class="mono">{d.id}</span>
            <span class="of">read as {d.geraten_als}, in {d.in_layouts.length} layouts</span>
            <pre class="mono">{d.vorschlag?.toml}</pre>
            {#if d.folgen}
              <span class="holds" class:tight={d.folgen.geaendert.length > 0}>
                {d.folgen.geaendert.length === 0
                  ? "changes no verdict"
                  : `changes ${d.folgen.geaendert.join(", ")}`}
              </span>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
{/if}

{#if !loading && !error && view === "inventory"}
  {#if wishlist}
    <section class="card budget">
      <h2>What is still missing</h2>
      <p class="sum">
        <strong>{euro(wishlist.summe_untere_kante_cent)}</strong>
        {#if wishlist.summe_obere_kante_cent > wishlist.summe_untere_kante_cent}
          <span class="upper">to {euro(wishlist.summe_obere_kante_cent)}</span>
        {/if}
        <span class="over">across {wishlist.items.length} open items</span>
      </p>
      {#if wishlist.posten_ohne_preis > 0}
        <p class="caveat">
          <Icon name="alert" size={14} />
          {wishlist.posten_ohne_preis} of them carry no price at all and count as zero — the
          total is a floor, not an estimate.
        </p>
      {/if}
      {#if wishlist.monatssaldo}
        <p class="finance">
          Finance measures a median monthly balance of
          <strong class:negative={wishlist.monatssaldo.median_cent < 0}>
            {euro(wishlist.monatssaldo.median_cent)}
          </strong>
          over {wishlist.monatssaldo.monate} complete months
          ({wishlist.monatssaldo.von} – {wishlist.monatssaldo.bis}).
          {#if wishlist.monate_bis_bezahlt !== null}
            That is <strong>{wishlist.monate_bis_bezahlt.toFixed(1)} months</strong> of surplus.
          {:else}
            Nothing accrues out of a balance that is not positive, so there is no number of
            months to give.
          {/if}
        </p>
      {:else}
        <p class="caveat">Finance has written nothing on this machine, so there is no balance to measure against.</p>
      {/if}
    </section>

    <h3 class="group">Wanted <span class="count">{wishlist.items.length}</span></h3>
    <ul class="items">
      {#each wishlist.items as i (i.id)}
        <li class="card item">
          <div class="head">
            <span class="label">{i.label}</span>
            {#if i.prioritaet}<span class="tag">{i.prioritaet}</span>{/if}
            {#if i.kind === "slot"}<span class="tag slot">slot</span>{/if}
          </div>
          {#if i.bild}
            <img class="shot" src={interior.mediaUrl(i.bild)} alt={i.label} loading="lazy" />
          {/if}
          <span class="dims mono">{size(i)}</span>
          <span class="price mono">
            {#if floorPrice(i) === null}
              <span class="unpriced">no price</span>
            {:else if i.preis_cent !== null}
              {euro(i.preis_cent)}
            {:else}
              {euro(i.kosten_min_cent ?? 0)} – {euro(i.kosten_max_cent ?? 0)}
            {/if}
          </span>
          {#if i.unsicher.length > 0}
            <span class="guess" title="measured? no.">~ {i.unsicher.join(", ")}</span>
          {/if}
          {#if i.ziel}<p class="target">{i.ziel}</p>{/if}
          <div class="actions">
            <button class="ghost" onclick={() => openEditor(i.id)}>Edit</button>
            <button class="ghost" disabled={saving} onclick={() => move(i.id, "owned")}>
              Mark bought
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  <h3 class="group">Owned <span class="count">{owned.length}</span></h3>
  <ul class="items">
    {#each owned as { item: i } (i.id)}
      <li class="card item">
        <div class="head">
          <span class="label">{i.label}</span>
          {#if i.mitnahme}<span class="tag">{i.mitnahme}</span>{/if}
        </div>
        {#if i.bild}
          <img class="shot" src={interior.mediaUrl(i.bild)} alt={i.label} loading="lazy" />
        {/if}
        <span class="dims mono">{size(i)}</span>
        {#if i.unsicher.length > 0}
          <span class="guess" title="measured? no.">~ {i.unsicher.join(", ")}</span>
        {:else if i.gemessen_am}
          <span class="measured mono">measured {i.gemessen_am.slice(0, 10)}</span>
        {/if}
        {#if declares(i)}
          <span class="declares mono" title="This piece states what it needs, so the name heuristic does not run for it (PRD Q61).">
            declares {declares(i)}
          </span>
        {/if}
        <div class="actions">
          <button class="ghost" onclick={() => openEditor(i.id)}>Edit</button>
          <button class="ghost" disabled={saving} onclick={() => move(i.id, "gone")}>
            Given away
          </button>
        </div>
      </li>
    {/each}
  </ul>

  {#if editing !== null}
    {@const row = byId.get(editing)}
    <div class="card editor">
      <h4>{row?.item.label ?? editing} <span class="mono id">{editing}</span></h4>

      {#if saveError}<p class="err">{saveError}</p>{/if}

      <div class="grid">
        <label>label <input bind:value={draft.label} /></label>
        <label>width cm <input bind:value={draft.b} inputmode="numeric" /></label>
        <label>depth cm <input bind:value={draft.t} inputmode="numeric" /></label>
        <label>height cm <input bind:value={draft.h} inputmode="numeric" /></label>
        <label>price ct <input bind:value={draft.preis_cent} inputmode="numeric" /></label>
        <label>priority <input bind:value={draft.prioritaet} /></label>
      </div>

      <label class="wide">
        picture <input bind:value={draft.bild} placeholder="produkt/…-01-….png" />
      </label>
      <p class="note">
        A path below the overlay's <code>media/</code> directory. Kept out of the bundle and
        fetched only when shown.
      </p>
      {#if draft.bild}
        <img class="shot big" src={interior.mediaUrl(String(draft.bild))} alt="" />
      {/if}
      <label class="wide">note <textarea rows="2" bind:value={draft.hinweis}></textarea></label>
      <label class="wide">reasoning <textarea rows="2" bind:value={draft.begruendung}></textarea></label>

      <h5>What this piece needs</h5>
      <p class="note">
        Fill any of these and the name heuristic stops judging this piece — it is checked against
        exactly what it states and nothing else (PRD Q61). Leave them empty and it is still
        measured by its name.
      </p>
      <div class="grid">
        <label>
          opens
          <select bind:value={draft.opens}>
            <option value="">— not stated —</option>
            <option value="nord">nord</option>
            <option value="sued">sued</option>
            <option value="ost">ost</option>
            <option value="west">west</option>
          </select>
        </label>
        <label>open_clear cm <input bind:value={draft.open_clear} inputmode="numeric" /></label>
        <label>
          wall_ok
          <select bind:value={draft.wall_ok}>
            <option value="">— not stated —</option>
            <option value="true">true — may sit against a wall</option>
            <option value="false">false — that side is useless there</option>
          </select>
        </label>
        <label>access_sides <input bind:value={draft.access_sides} inputmode="numeric" /></label>
        <label>access_clear cm <input bind:value={draft.access_clear} inputmode="numeric" /></label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.raumtrenner} />
          raumtrenner — meant to stand free
        </label>
      </div>

      <div class="actions">
        <button class="ghost" disabled={impactBusy} onclick={preview}>
          {impactBusy ? "Checking…" : "Check impact"}
        </button>
        <button disabled={saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
        <button class="ghost" onclick={() => { editing = null; impact = null; }}>Close</button>
      </div>

      <!--
        The reason this is a button and not a surprise. Declaring which side the wardrobe opens
        costs 2 or 4 layouts depending on the direction, and nothing said so until it was worked
        out by hand. The capability re-checks every layout here; the page only prints the answer.
      -->
      {#if impact}
        <div class="impact" class:worse={impact.bestanden_nachher < impact.bestanden_vorher}>
          <strong>
            {impact.bestanden_vorher} → {impact.bestanden_nachher} of {impact.layouts} layouts pass
          </strong>
          {#if impact.geaendert.length === 0}
            <p>No verdict moves. Safe to save.</p>
          {:else}
            <ul>
              {#each impact.geaendert as g (g.layout)}
                <li>
                  <span class="mono">{g.layout}</span>
                  {g.vorher.pass ? "passes" : "fails"} → {g.nachher.pass ? "passes" : "fails"}
                  {#if g.nachher.hard.length > 0}
                    <span class="mono rules">{g.nachher.hard.join(", ")}</span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      <h5>State history</h5>
      {#if history.length === 0}
        <p class="note">No history recorded.</p>
      {:else}
        <ul class="history">
          {#each history as h, n (h.since + n)}
            <li>
              <span class="mono st">{h.state}</span>
              <span class="mono when">{h.since.slice(0, 10)}</span>
              {#if h.note}<span class="why">{h.note}</span>{/if}
            </li>
          {/each}
        </ul>
        <p class="note">
          Appended, never overwritten — a wish that gets bought is a second row, and that span is
          what the wishlist joins to money with.
        </p>
      {/if}
    </div>
  {/if}

  <div class="addrow">
    <button class="ghost" onclick={() => (creating = !creating)}>
      {creating ? "Cancel" : "+ New entry"}
    </button>
  </div>

  {#if creating}
    <form class="card editor" onsubmit={(e) => { e.preventDefault(); create(); }}>
      <h4>New entry</h4>
      <div class="grid">
        <label>id <input bind:value={draftNew.id} placeholder="stehlampe" required /></label>
        <label>label <input bind:value={draftNew.label} placeholder="IKEA …" required /></label>
        <label>
          kind
          <select bind:value={draftNew.kind}>
            <option value="piece">piece — a thing, or a specific product</option>
            <option value="slot">slot — a need with target sizes, no product yet</option>
          </select>
        </label>
        <label>
          state
          <select bind:value={draftNew.state}>
            <option value="wanted">wanted</option>
            <option value="owned">owned</option>
          </select>
        </label>
        <label>width cm <input bind:value={draftNew.b} inputmode="numeric" /></label>
        <label>depth cm <input bind:value={draftNew.t} inputmode="numeric" /></label>
        <label>height cm <input bind:value={draftNew.h} inputmode="numeric" /></label>
        <label>price ct <input bind:value={draftNew.preis} inputmode="numeric" /></label>
      </div>
      <p class="note">
        State is required, not a default — an entry with no state joins to nothing and would
        never appear in any list.
      </p>
      <div class="actions">
        <button type="submit" disabled={saving || !draftNew.id.trim() || !draftNew.label.trim()}>
          {saving ? "Creating…" : "Create"}
        </button>
      </div>
    </form>
  {/if}

  {#if gone.length > 0}
    <h3 class="group">Gone <span class="count">{gone.length}</span></h3>
    <ul class="items">
      {#each gone as { item: i } (i.id)}
        <li class="card item muted">
          <div class="head"><span class="label">{i.label}</span></div>
          <span class="dims mono">{size(i)}</span>
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .views {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0 1.25rem;
  }
  .views button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 0.8125rem;
    padding: 0.4rem 0.85rem;
  }
  .views button.active {
    border-color: var(--primary);
    color: var(--text-primary);
  }
  .count {
    color: var(--text-tertiary);
    margin-left: 0.35rem;
  }

  .plans {
    display: grid;
    gap: 1.25rem;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
  }
  @media (max-width: 60rem) {
    .plans {
      grid-template-columns: 1fr;
    }
  }

  .variants {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .variants button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: grid;
    gap: 0.15rem;
    padding: 0.6rem 0.75rem;
    text-align: left;
    width: 100%;
  }
  .variants button.active {
    border-color: var(--primary);
  }
  .variants .name {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .variants .tally,
  .measured {
    color: var(--text-tertiary);
    font-size: 0.75rem;
  }
  .verdict {
    color: var(--danger);
    font-size: 0.6875rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .verdict.pass {
    color: var(--success);
  }

  /* Which plan, in which room. Above the drawing, because that is where the eye starts. */
  .where {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
  }
  .where strong {
    color: var(--text-primary);
    font-size: 0.9375rem;
  }
  .where .id {
    color: var(--text-tertiary);
    font-size: 0.75rem;
  }

  /* The list of plans and the way to add one are one grid column, not two. */
  .side {
    display: flex;
    flex-direction: column;
  }

  .newplan {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-top: 0.6rem;
  }
  .newplan button {
    background: transparent;
    border: 1px dashed var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.35rem 0.7rem;
  }
  .newplan button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .newplan input:not([type="checkbox"]) {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.45rem;
    width: 100%;
  }
  .newplan label {
    align-items: center;
    color: var(--text-tertiary);
    display: flex;
    font-size: 0.72rem;
    gap: 0.35rem;
  }
  .newplan .bad {
    color: var(--warning);
    font-size: 0.72rem;
    margin: 0;
  }

  /* The measurements of one piece, while the pointer is on it. */
  .piece {
    align-items: baseline;
    border-top: 1px solid var(--card-border);
    display: flex;
    flex-wrap: wrap;
    font-size: 0.78rem;
    gap: 0.15rem 0.7rem;
    margin: 0.6rem 0 0;
    padding-top: 0.45rem;
  }
  .piece .ref {
    color: var(--primary);
  }
  .piece .what {
    color: var(--text-secondary);
  }
  .piece strong {
    color: var(--text-primary);
  }
  /* What the piece states it needs, in the vocabulary the file uses for it (PRD Q61). */
  .piece .needs {
    color: var(--warning);
    font-size: 0.72rem;
  }

  .planbar button.armed {
    border-color: var(--warning);
    color: var(--warning);
  }

  .plan .svg :global(svg) {
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    height: auto;
    max-width: 100%;
  }

  .metrics {
    display: flex;
    flex-wrap: wrap;
    gap: 1.25rem;
    margin: 0.85rem 0 0;
  }
  .metrics div {
    display: flex;
    gap: 0.4rem;
  }
  .metrics dt {
    color: var(--text-tertiary);
  }
  .metrics dd {
    color: var(--text-primary);
    margin: 0;
  }

  .corridors {
    border-collapse: collapse;
    margin-top: 0.85rem;
    width: 100%;
  }
  .corridors caption {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    padding-bottom: 0.35rem;
    text-align: left;
  }
  .corridors th {
    color: var(--text-secondary);
    font-weight: 400;
    text-align: left;
  }
  .corridors th,
  .corridors td {
    border-top: 1px solid var(--card-border);
    font-size: 0.8125rem;
    padding: 0.3rem 0;
  }
  .corridors td {
    text-align: right;
  }

  .violations {
    display: grid;
    gap: 0.3rem;
    list-style: none;
    margin: 0.85rem 0 0;
    padding: 0;
  }
  .violations li {
    border-left: 2px solid var(--danger);
    color: var(--text-secondary);
    font-size: 0.8125rem;
    padding-left: 0.6rem;
  }
  .violations.soft li {
    border-left-color: var(--warning);
  }
  .rule {
    color: var(--text-tertiary);
    margin-right: 0.4rem;
  }

  /* The rule's own wording, under the violation it explains. Quiet: the reader came for what
     is wrong, and stays for why it counts as wrong. */
  .rule-text {
    color: var(--text-tertiary);
    display: block;
    font-size: 0.75rem;
    margin-top: 0.15rem;
  }

  /* ─── Dragging ────────────────────────────────────────────────────────── */

  .plan .svg :global(g[data-ref]) {
    cursor: grab;
  }

  /* No text selection while a piece is being moved. */
  .plan .svg.dragging {
    user-select: none;
  }

  .plan .svg :global(g[data-ref]:hover rect) {
    stroke: var(--primary);
  }

  .plan .svg :global(g[data-ref].picked rect) {
    stroke: var(--primary);
    stroke-width: 6;
  }

  .planbar {
    align-items: center;
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    margin-top: 0.7rem;
  }

  .planbar button {
    background: transparent;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.7rem;
  }

  .planbar button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }

  .planbar input {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.45rem;
    width: 9rem;
  }

  .planbar .moved {
    color: var(--warning);
    font-size: 0.75rem;
    margin-right: 0.2rem;
  }

  /* ─── Editing ─────────────────────────────────────────────────────────── */

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.6rem;
  }

  .actions button {
    background: var(--primary);
    border: 1px solid transparent;
    border-radius: 6px;
    color: var(--card-bg);
    cursor: pointer;
    font: inherit;
    font-size: 0.75rem;
    padding: 0.3rem 0.7rem;
  }

  .actions button.ghost {
    background: transparent;
    border-color: var(--card-border);
    color: var(--text-secondary);
  }

  .actions button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .addrow {
    margin: 1rem 0;
  }

  .addrow button {
    background: transparent;
    border: 1px dashed var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.45rem 0.9rem;
  }

  .editor {
    margin: 1rem 0 1.5rem;
    padding: 1rem;
  }

  .editor h4 {
    align-items: baseline;
    display: flex;
    gap: 0.5rem;
    margin: 0 0 0.75rem;
  }

  .editor h5 {
    margin: 1.1rem 0 0.3rem;
  }

  .editor .id {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    font-weight: 400;
  }

  .editor .grid {
    display: grid;
    gap: 0.6rem;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  }

  .editor label {
    color: var(--text-tertiary);
    display: flex;
    flex-direction: column;
    font-size: 0.7rem;
    gap: 0.2rem;
    text-transform: uppercase;
  }

  .editor label.wide {
    margin-top: 0.6rem;
  }

  .editor label.check {
    align-items: center;
    flex-direction: row;
    text-transform: none;
  }

  .editor input,
  .editor select,
  .editor textarea {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.35rem 0.45rem;
    text-transform: none;
    width: 100%;
  }

  .editor .note {
    color: var(--text-tertiary);
    font-size: 0.72rem;
    margin: 0.35rem 0 0;
  }

  .editor .err {
    color: var(--danger, #f87171);
    font-size: 0.78rem;
    margin: 0 0 0.6rem;
  }

  /* The arithmetic that used to have to be done by hand, one direction at a time. */
  .impact {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-left: 3px solid var(--primary);
    border-radius: 6px;
    font-size: 0.78rem;
    margin-top: 0.85rem;
    padding: 0.6rem 0.75rem;
  }

  .impact.worse {
    border-left-color: var(--warning);
  }

  .impact ul,
  .history {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
  }

  .impact li,
  .history li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0.15rem 0;
  }

  .impact .rules {
    color: var(--warning);
  }

  .history {
    font-size: 0.78rem;
  }

  .history .st {
    color: var(--primary);
    min-width: 4.5rem;
  }

  .history .when {
    color: var(--text-tertiary);
  }

  .history .why {
    color: var(--text-secondary);
  }

  .shot {
    border: 1px solid var(--card-border);
    border-radius: 6px;
    display: block;
    margin: 0.4rem 0;
    max-height: 8rem;
    object-fit: cover;
    width: 100%;
  }

  .shot.big {
    max-height: 18rem;
    object-fit: contain;
  }

  .declares {
    color: var(--primary);
    font-size: 0.7rem;
  }

  .guessed,
  .caveat,
  .unchecked {
    align-items: center;
    color: var(--warning);
    display: flex;
    font-size: 0.75rem;
    gap: 0.35rem;
    margin: 0.85rem 0 0;
  }

  .budget h2 {
    font-size: 0.9375rem;
    margin: 0 0 0.5rem;
  }
  .budget .sum {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0;
  }
  .budget .sum strong {
    color: var(--text-primary);
    font-size: 1.5rem;
  }
  .upper,
  .over {
    color: var(--text-tertiary);
    font-size: 0.8125rem;
  }
  .finance {
    color: var(--text-secondary);
    font-size: 0.8125rem;
    margin: 0.75rem 0 0;
  }
  .finance strong {
    color: var(--success);
  }
  .finance strong.negative {
    color: var(--danger);
  }

  .group {
    align-items: baseline;
    display: flex;
    font-size: 0.75rem;
    gap: 0.35rem;
    letter-spacing: 0.06em;
    margin: 1.5rem 0 0.6rem;
    text-transform: uppercase;
  }

  .items {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .item {
    display: grid;
    gap: 0.25rem;
    padding: 0.7rem 0.85rem;
  }
  .item.muted {
    opacity: 0.55;
  }
  .item .head {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .item .label {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .tag {
    background: var(--primary-soft);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 0.6875rem;
    padding: 0.05rem 0.35rem;
  }
  .tag.slot {
    background: var(--warning-soft);
  }
  .dims,
  .price {
    color: var(--text-secondary);
    font-size: 0.8125rem;
  }
  .unpriced {
    color: var(--warning);
  }
  .guess {
    color: var(--warning);
    font-size: 0.75rem;
  }
  .target {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    margin: 0.25rem 0 0;
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
  }
  .offer,
  .budget {
    padding: 1rem 1.15rem;
  }
  .offer .lead {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
    margin: 0 0 0.75rem;
  }
  .offer button {
    background: var(--primary);
    border: none;
    border-radius: var(--radius-sm);
    color: white;
    cursor: pointer;
    font-size: 0.8125rem;
    padding: 0.45rem 0.9rem;
  }
  .empty,
  .error {
    color: var(--text-tertiary);
    font-size: 0.8125rem;
  }
  .error {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* By how much, not whether. */
  .reserve {
    color: var(--text-secondary);
    font-size: 0.8125rem;
    margin: 0.85rem 0 0;
  }
  .reserve.tight {
    color: var(--warning);
  }
  .reserve strong {
    color: var(--text-primary);
  }
  .holds,
  .of {
    color: var(--text-tertiary);
    font-size: 0.75rem;
  }
  .holds.tight {
    color: var(--warning);
  }

  .reserves {
    margin-top: 0.85rem;
  }
  .reserves summary {
    color: var(--text-tertiary);
    cursor: pointer;
    font-size: 0.75rem;
  }
  .reserves .slack {
    text-align: right;
  }
  .reserves .slack.neg {
    color: var(--danger);
  }
  .reserves tr.tight th,
  .reserves tr.tight td {
    color: var(--warning);
  }

  .movein {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
  }
  .movein li {
    border-top: 1px solid var(--card-border);
    display: grid;
    font-size: 0.75rem;
    gap: 0.5rem;
    grid-template-columns: 10rem 5rem 1fr 1fr;
    padding: 0.35rem 0;
  }
  .movein li.bad {
    color: var(--danger);
  }
  .movein .dims,
  .movein .way {
    color: var(--text-tertiary);
  }

  /* One row per day, one column per hour: a year on four lines. */
  .sun {
    border-collapse: collapse;
    font-size: 0.75rem;
    margin-top: 0.5rem;
  }
  .sun th {
    color: var(--text-tertiary);
    font-weight: 400;
    padding-right: 0.75rem;
    text-align: left;
  }
  .sun td {
    border: 1px solid var(--card-border);
    color: var(--text-tertiary);
    min-width: 1.5rem;
    padding: 0.15rem 0.3rem;
    text-align: center;
  }
  .sun td.night {
    background: var(--surface);
    color: var(--text-tertiary);
  }
  .sun td.hit {
    background: color-mix(in srgb, var(--warning) 22%, transparent);
    color: var(--text-primary);
  }
  .sunlist {
    color: var(--text-secondary);
    font-size: 0.75rem;
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
  }

  .solver h2 {
    font-size: 0.9375rem;
    margin: 0 0 0.5rem;
  }
  .picks {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    margin: 0.6rem 0;
  }
  .picks label,
  .probe label,
  .stepper {
    align-items: center;
    color: var(--text-secondary);
    display: inline-flex;
    font-size: 0.8125rem;
    gap: 0.35rem;
  }
  .stepper input,
  .probe input[type="number"] {
    width: 4.5rem;
  }
  .probe {
    align-items: center;
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin: 0.6rem 0;
  }

  .composed {
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0;
  }
  .composed h3 {
    font-size: 0.8125rem;
    margin: 0 0 0.3rem;
  }
  .places {
    color: var(--text-tertiary);
    font-size: 0.75rem;
    list-style: none;
    margin: 0.35rem 0 0;
    padding: 0;
  }

  .declarations {
    list-style: none;
    margin: 0.6rem 0 0;
    padding: 0;
  }
  .declarations li {
    border-top: 1px solid var(--card-border);
    padding: 0.5rem 0;
  }
  .declarations pre {
    background: var(--surface);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 0.75rem;
    margin: 0.35rem 0;
    padding: 0.4rem 0.6rem;
    white-space: pre-wrap;
  }
</style>
