<script lang="ts">
  /**
   * People: the entity core's person kind (capabilities/entities, PRD Q117).
   *
   * Axon is the system of record; a person's note in Obsidian holds the prose and is
   * linked, not copied. Fields render from the registry, so a field declared here needs no
   * code change to appear. Every value and fact is C2 and stays on this machine.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { entities, type Entity, type EntityField } from "$lib/api";

  let people = $state<Entity[]>([]);
  let fields = $state<EntityField[]>([]);
  let query = $state("");
  let selectedId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);

  const today = new Date().toISOString().slice(0, 10);
  const selected = $derived(people.find((p) => p.id === selectedId) ?? null);
  const filtered = $derived(
    query.trim()
      ? people.filter((p) => p.name.toLowerCase().includes(query.trim().toLowerCase()))
      : people,
  );

  /** Where a person is today: an away period covering it, else the home base that holds. */
  function whereToday(person: Entity): string | null {
    const covers = (f: Entity["facts"][number]) =>
      (!f.valid_from || f.valid_from <= today) && (!f.valid_to || f.valid_to >= today);
    const latest = (predicate: string) =>
      person.facts
        .filter((f) => f.predicate === predicate && covers(f))
        .sort((a, b) => (b.valid_from ?? "").localeCompare(a.valid_from ?? ""))[0];
    const fact = latest("away") ?? latest("home_base");
    return fact ? (fact.predicate === "away" ? `in ${fact.place}` : fact.place) : null;
  }

  async function load(): Promise<void> {
    try {
      [people, fields] = await Promise.all([entities.list("person"), entities.fields("person")]);
      error = null;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  onMount(() => void load());

  function replace(updated: Entity): void {
    people = people.map((p) => (p.id === updated.id ? updated : p));
  }

  async function run(action: () => Promise<void>, done?: string): Promise<void> {
    busy = true;
    error = null;
    notice = null;
    try {
      await action();
      if (done) notice = done;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busy = false;
    }
  }

  // ─── Add a person ───
  let newName = $state("");
  function addPerson(event: SubmitEvent): void {
    event.preventDefault();
    const name = newName.trim();
    if (!name) return;
    void run(async () => {
      const created = await entities.create({ kind: "person", name });
      people = [...people, created].sort((a, b) => a.name.localeCompare(b.name));
      selectedId = created.id;
      newName = "";
    });
  }

  // ─── Field values: a draft per selected person, saved in one PATCH ───
  let draft = $state<Record<string, string>>({});
  $effect(() => {
    const person = selected;
    const next: Record<string, string> = {};
    for (const field of fields) {
      const value = person?.values[field.key]?.value;
      next[field.key] = Array.isArray(value) ? value.join(", ") : value == null ? "" : String(value);
    }
    draft = next;
  });

  /** A draft string as the value its field type takes; null clears. */
  function parsed(field: EntityField, raw: string): unknown {
    const text = raw.trim();
    if (!text) return null;
    if (field.field_type === "emails" || field.field_type === "phones") {
      return text.split(",").map((s) => s.trim()).filter(Boolean);
    }
    if (field.field_type === "bool") return text === "true";
    if (field.field_type === "number") return Number(text);
    return text;
  }

  function saveFields(event: SubmitEvent): void {
    event.preventDefault();
    const person = selected;
    if (!person) return;
    const values: Record<string, unknown> = {};
    for (const field of fields) {
      const before = person.values[field.key]?.value;
      const beforeText = Array.isArray(before) ? before.join(", ") : before == null ? "" : String(before);
      if ((draft[field.key] ?? "") !== beforeText) values[field.key] = parsed(field, draft[field.key] ?? "");
    }
    if (Object.keys(values).length === 0) return;
    void run(async () => {
      replace(await entities.patch(person.id, { values, expected_revision: person.revision }));
    }, "Saved.");
  }

  // ─── Dated facts ───
  let factPredicate = $state<"home_base" | "away">("away");
  let factPlace = $state("");
  let factFrom = $state("");
  let factTo = $state("");
  let factNote = $state("");
  function addFact(event: SubmitEvent): void {
    event.preventDefault();
    const person = selected;
    if (!person || !factPlace.trim()) return;
    void run(async () => {
      const added = await entities.addFact(person.id, {
        predicate: factPredicate,
        place: factPlace.trim(),
        valid_from: factFrom || undefined,
        valid_to: factTo || undefined,
        note: factNote.trim() || undefined,
      });
      replace(await entities.get(person.id));
      factPlace = "";
      factFrom = "";
      factTo = "";
      factNote = "";
      if (added.geocode.status !== "found") {
        notice = `Saved, but no coordinate for "${added.fact.place}" (${added.geocode.status}). It will not match any trip leg.`;
      }
    });
  }

  function removeFact(factId: string): void {
    const person = selected;
    if (!person) return;
    void run(async () => {
      await entities.removeFact(person.id, factId);
      replace(await entities.get(person.id));
    });
  }

  // ─── Declare a field ───
  let fieldLabel = $state("");
  let fieldType = $state<EntityField["field_type"]>("text");
  let fieldOptions = $state("");
  const keyOf = (label: string) =>
    label.trim().toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").replace(/^(\d)/, "f_$1").slice(0, 40);
  function declareField(event: SubmitEvent): void {
    event.preventDefault();
    const label = fieldLabel.trim();
    if (!label) return;
    void run(async () => {
      const field = await entities.declareField({
        kind: "person",
        key: keyOf(label),
        label,
        field_type: fieldType,
        options: fieldType === "enum" ? fieldOptions.split(",").map((o) => o.trim()).filter(Boolean) : [],
        data_class: "C2",
      });
      fields = [...fields, field];
      fieldLabel = "";
      fieldOptions = "";
    }, `Field "${label}" added.`);
  }

  function period(fact: Entity["facts"][number]): string {
    if (fact.predicate === "home_base") return fact.valid_from ? `since ${fact.valid_from}` : "home base";
    return `${fact.valid_from} – ${fact.valid_to}`;
  }
</script>

<PageHeader badge="People" title="People you know" desc="Where they live, where they are, and where you could stay. Stored in Axon; notes stay in Obsidian." />

{#if error}<p class="error"><Icon name="alert" size={15} /> {error}</p>{/if}
{#if notice}<p class="notice" aria-live="polite">{notice}</p>{/if}

<div class="people">
  <section class="list" aria-label="People">
    <input class="search" aria-label="Search people" placeholder="Search" bind:value={query} />
    <ol>
      {#each filtered as person (person.id)}
        <li>
          <button type="button" class:active={person.id === selectedId} onclick={() => (selectedId = person.id)}>
            <strong>{person.name}</strong>
            <span>{whereToday(person) ?? ""}</span>
          </button>
        </li>
      {/each}
    </ol>
    <form class="row" onsubmit={addPerson}>
      <input aria-label="New person's name" placeholder="New person" bind:value={newName} />
      <button class="btn btn-outline" type="submit" disabled={busy}><Icon name="plus" size={13} /> Add</button>
    </form>
  </section>

  <section class="detail" aria-label="Person">
    {#if !selected}
      <p class="empty">Pick a person, or add one.</p>
    {:else}
      <header>
        <h2>{selected.name}</h2>
        <p class="meta">
          {whereToday(selected) ? `Today: ${whereToday(selected)}` : "No home base yet"}
          {#if selected.note_ref} · note: {selected.note_ref}{/if}
        </p>
      </header>

      <h3>Where</h3>
      {#if selected.facts.length === 0}
        <p class="empty">No home base or away periods yet.</p>
      {:else}
        <ol class="facts">
          {#each selected.facts as fact (fact.id)}
            <li>
              <span class="tag">{fact.predicate === "away" ? "Away" : "Home"}</span>
              <strong>{fact.place}</strong>
              <span class="meta">{period(fact)}{fact.note ? ` · ${fact.note}` : ""}{fact.latitude == null ? " · no coordinate" : ""}</span>
              <button class="link" type="button" aria-label={`Delete ${fact.place}`} onclick={() => removeFact(fact.id)}>
                <Icon name="close" size={12} />
              </button>
            </li>
          {/each}
        </ol>
      {/if}
      <form class="fact-form" onsubmit={addFact}>
        <select aria-label="Kind of fact" bind:value={factPredicate}>
          <option value="away">Away</option>
          <option value="home_base">Home base</option>
        </select>
        <input aria-label="Place" placeholder="City" bind:value={factPlace} required />
        <input aria-label="From" type="date" bind:value={factFrom} required={factPredicate === "away"} />
        <input aria-label="To" type="date" bind:value={factTo} required={factPredicate === "away"} disabled={factPredicate === "home_base"} />
        <input aria-label="Note" placeholder="Note" bind:value={factNote} />
        <button class="btn btn-outline" type="submit" disabled={busy}>Add</button>
      </form>

      <h3>Details</h3>
      <form class="fields" onsubmit={saveFields}>
        {#each fields as field (field.key)}
          <label>
            <span>{field.label}</span>
            {#if field.field_type === "enum"}
              <select aria-label={field.label} bind:value={draft[field.key]}>
                <option value="">–</option>
                {#each field.options as option (option)}<option value={option}>{option}</option>{/each}
              </select>
            {:else if field.field_type === "bool"}
              <select aria-label={field.label} bind:value={draft[field.key]}>
                <option value="">–</option><option value="true">yes</option><option value="false">no</option>
              </select>
            {:else}
              <input
                aria-label={field.label}
                type={field.field_type === "date" ? "date" : field.field_type === "number" ? "number" : "text"}
                placeholder={field.field_type === "emails" || field.field_type === "phones" ? "comma-separated" : ""}
                bind:value={draft[field.key]}
              />
            {/if}
          </label>
        {/each}
        <button class="btn btn-primary" type="submit" disabled={busy}>Save details</button>
      </form>

      <details class="declare">
        <summary>Add a field for everyone</summary>
        <form class="fact-form" onsubmit={declareField}>
          <input aria-label="Field label" placeholder="Label, e.g. Climbing grade" bind:value={fieldLabel} required />
          <select aria-label="Field type" bind:value={fieldType}>
            {#each ["text", "enum", "bool", "date", "number", "url", "emails", "phones"] as type (type)}
              <option value={type}>{type}</option>
            {/each}
          </select>
          {#if fieldType === "enum"}
            <input aria-label="Options" placeholder="Options, comma-separated" bind:value={fieldOptions} required />
          {/if}
          <button class="btn btn-outline" type="submit" disabled={busy}>Add field</button>
        </form>
      </details>
    {/if}
  </section>
</div>

<style>
  .people {
    display: grid;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
    gap: 1rem;
    align-items: start;
  }

  @media (max-width: 760px) {
    .people {
      grid-template-columns: 1fr;
    }
  }

  .list,
  .detail {
    padding: 0.9rem 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
  }

  .list ol,
  .facts {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin: 0.6rem 0;
    padding: 0;
    list-style: none;
  }

  .list ol {
    max-height: 60vh;
    overflow-y: auto;
  }

  .list li button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.35rem 0.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .list li button span {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .list li button.active,
  .list li button:hover {
    background: var(--primary-soft);
  }

  input,
  select {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
  }

  .search {
    width: 100%;
  }

  .row {
    display: flex;
    gap: 0.4rem;
  }

  .row input {
    flex: 1;
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: var(--text-lg);
  }

  h3 {
    margin: 1rem 0 0.3rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .meta,
  .empty {
    margin: 0.2rem 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .facts li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.45rem;
    font-size: var(--text-sm);
  }

  .tag {
    padding: 0 0.35rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-2xs);
  }

  .link {
    margin-left: auto;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    cursor: pointer;
  }

  .fact-form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.4rem;
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: 0.6rem;
  }

  .fields label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .fields button {
    align-self: end;
  }

  .declare {
    margin-top: 1rem;
    font-size: var(--text-sm);
  }

  .error,
  .notice {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-sm);
  }

  .error {
    color: var(--danger);
  }

  .notice {
    color: var(--text-secondary);
  }
</style>
