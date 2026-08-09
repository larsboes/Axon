<script lang="ts">
  import { onMount } from "svelte";
  import { base } from "$app/paths";
  import { page } from "$app/state";
  import "../app.css";
  import Icon from "$lib/Icon.svelte";
  import { capabilities } from "$lib/capabilities.svelte";
  import { PRIMARY_NAV, UTILITY_NAV, withoutCapabilities, link } from "$lib/nav";
  import SoundscapeDock from "$lib/SoundscapeDock.svelte";

  let { children, data } = $props();

  let dark = $state(true);
  let menuOpen = $state(false);
  let moreOpen = $state(false);
  let now = $state(new Date());

  // `data.demo` is null outside a demo build, so both lists below are the untouched arrays
  // and the banner never renders. In a demo build the index names the capabilities the
  // recording could not include; the destinations that lead only to those are dropped
  // rather than left to render a page of error cards (#168).
  const demo = $derived(data?.demo ?? null);
  const missing = $derived(new Set(Object.keys(demo?.absent ?? {})));
  const primary = $derived(withoutCapabilities(PRIMARY_NAV, missing));
  const utility = $derived(withoutCapabilities(UTILITY_NAV, missing));

  const isActive = (href: string) =>
    href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  const utilityActive = $derived(utility.some((item) => isActive(item.href)));

  onMount(() => {
    dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    const clock = setInterval(() => (now = new Date()), 1000);
    const stop = capabilities.subscribe();
    return () => {
      clearInterval(clock);
      stop();
    };
  });

  $effect(() => {
    document.documentElement.classList.toggle("dark", dark);
  });
</script>

<div class="shell">
  <header>
    <div class="bar">
      <a class="brand" href={link("/")}>
        <span class="mark">A</span>
        <span class="name">Axon</span>
      </a>

      <div class="meta">
        <span class="clock mono">
          <Icon name="clock" size={14} />
          {now.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })}
        </span>
        <button class="btn" onclick={() => (dark = !dark)} aria-label="Switch theme">
          <Icon name={dark ? "sun" : "moon"} />
        </button>
        <button
          class="btn burger"
          onclick={() => (menuOpen = !menuOpen)}
          aria-label="Menu"
          aria-expanded={menuOpen}
        >
          <Icon name={menuOpen ? "close" : "menu"} />
        </button>
      </div>
    </div>

    <div class="desktop-wrap">
      <nav class="desktop" aria-label="Main navigation">
        {#each primary as item (item.href)}
          <a class="nav-link" class:active={isActive(item.href)} href={link(item.href)}>
            <Icon name={item.icon as never} size={14} />
            {item.label}
          </a>
        {/each}
      </nav>

      <details class="more" bind:open={moreOpen}>
        <summary class="nav-link" class:active={utilityActive}>
          <Icon name="boxes" size={14} />
          More
        </summary>
        <nav class="more-menu" aria-label="Projects and system">
          {#each utility as item (item.href)}
            <a
              class="nav-link"
              class:active={isActive(item.href)}
              href={link(item.href)}
              onclick={() => (moreOpen = false)}
            >
              <Icon name={item.icon as never} size={14} />
              {item.label}
            </a>
          {/each}
        </nav>
      </details>
    </div>
  </header>

  {#if demo}
    <!-- Sticky under the header rather than dismissible: a visitor who scrolls past a
         one-time notice and then reads a balance is exactly who this is for. -->
    <aside class="demo-banner">
      <strong>{demo.label}</strong>
      <span>
        Every figure below was generated from seed <code>{demo.seed}</code> and dated around
        {demo.anchor}. Writing is disabled.
      </span>
      {#if missing.size > 0}
        <span class="demo-absent">Not in this demo: {[...missing].join(", ")}</span>
      {/if}
      <!-- The generated reference is a sibling of this bundle, not a route in it, so it is a
           plain href rather than link(): the router must not try to handle it. -->
      <a class="demo-docs" href="{base}/docs/index.html">Reference →</a>
    </aside>
  {/if}

  {#if menuOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="scrim" onclick={() => (menuOpen = false)}></div>
    <nav class="mobile" aria-label="Mobile navigation">
      <span class="nav-section">Work</span>
      {#each primary as item (item.href)}
        <a
          class="nav-link"
          class:active={isActive(item.href)}
          href={link(item.href)}
          onclick={() => (menuOpen = false)}
        >
          <Icon name={item.icon as never} />
          {item.label}
        </a>
      {/each}
      <span class="nav-section second">Projects and system</span>
      {#each utility as item (item.href)}
        <a
          class="nav-link"
          class:active={isActive(item.href)}
          href={link(item.href)}
          onclick={() => (menuOpen = false)}
        >
          <Icon name={item.icon as never} />
          {item.label}
        </a>
      {/each}
    </nav>
  {/if}

  <main>
    {@render children()}
  </main>

  <footer>
    <div class="inner">
      <span>Axon</span>
      <span class="mono">{capabilities.items.length} capabilities</span>
    </div>
  </footer>

  <SoundscapeDock />
</div>

<style>
  .shell {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    /* Set by SoundscapeDock while it is mounted, absent otherwise. */
    padding-bottom: var(--soundscape-dock-height, 0px);
  }

  header {
    position: sticky;
    top: 0;
    z-index: 50;
    background-color: var(--header-bg);
    backdrop-filter: blur(12px);
    border-bottom: 1px solid var(--header-border);
  }

  /* `min(100%, …)` rather than a bare cap: below --shell-max the shell IS the viewport,
   * so the layout runs to the edges (minus each band's own padding) instead of sitting in
   * a column with gutters. The cap only takes over on a display wide enough that
   * full-bleed would stretch a line of text past reading comfort. See app.css for why
   * this is px and the type scale is not. */
  .bar,
  .desktop-wrap,
  main,
  footer .inner {
    max-width: min(100%, var(--shell-max));
    margin-inline: auto;
    width: 100%;
  }

  .bar {
    height: 3.5rem;
    padding-inline: 1.5rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .mark {
    display: grid;
    place-items: center;
    height: 1.75rem;
    width: 1.75rem;
    border-radius: var(--radius-md);
    background-color: var(--primary);
    color: var(--text-inverse);
    font-size: 0.65rem;
    font-weight: 700;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--text-tertiary);
    font-size: 0.75rem;
  }

  .clock {
    display: none;
    align-items: center;
    gap: 0.4rem;
    font-variant-numeric: tabular-nums;
  }

  .desktop-wrap {
    display: none;
    align-items: center;
    justify-content: space-between;
    padding: 0 1.5rem 0.25rem;
  }

  nav.desktop {
    display: flex;
    gap: 0.125rem;
  }

  .nav-link {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem;
    font-size: 0.75rem;
    font-weight: 500;
    border-radius: var(--radius-md);
    color: var(--nav-inactive);
    transition:
      color 0.15s ease,
      background-color 0.15s ease;
  }

  .nav-link:hover {
    color: var(--nav-hover);
    background-color: var(--primary-soft);
  }

  .nav-link.active {
    color: var(--primary);
    background-color: var(--primary-soft);
  }

  .more {
    position: relative;
  }

  .more summary {
    list-style: none;
    cursor: pointer;
  }

  .more summary::-webkit-details-marker {
    display: none;
  }

  .more-menu {
    position: absolute;
    top: calc(100% + 0.4rem);
    right: 0;
    width: 13rem;
    padding: 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--card-shadow-hover);
  }

  .more-menu .nav-link {
    width: 100%;
  }

  .demo-banner {
    position: sticky;
    top: 3.5rem;
    z-index: 45;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.25rem 0.75rem;
    padding: 0.5rem clamp(1rem, 2.5vw, 1.5rem);
    border-bottom: 1px solid var(--card-border);
    /* colour-mix against the page rather than a new token: the banner has to read as a
       notice in both themes without app.css growing a palette entry only one build uses. */
    background: color-mix(in oklab, var(--primary-soft) 70%, var(--page-bg));
    color: var(--text-secondary);
    font-size: 0.75rem;
    line-height: 1.5;
  }

  .demo-banner strong {
    color: var(--primary);
  }

  .demo-absent {
    color: var(--text-tertiary);
  }

  .demo-banner .demo-docs {
    margin-left: auto;
    color: var(--primary);
    white-space: nowrap;
  }

  .scrim {
    position: fixed;
    inset: 3.5rem 0 0;
    z-index: 40;
    background-color: rgb(0 0 0 / 40%);
  }

  nav.mobile {
    position: fixed;
    inset: 3.5rem 0 auto auto;
    z-index: 41;
    height: calc(100vh - 3.5rem);
    width: 16rem;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    background-color: var(--page-bg);
    border-left: 1px solid var(--header-border);
  }

  nav.mobile .nav-link {
    padding: 0.75rem 1rem;
    font-size: 0.875rem;
  }

  .nav-section {
    padding: 0.35rem 1rem 0.2rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .nav-section.second {
    margin-top: 0.75rem;
    padding-top: 1rem;
    border-top: 1px solid var(--card-border);
  }

  main {
    flex-grow: 1;
    padding: clamp(1.25rem, 2.5vw, 2.5rem);
  }

  footer {
    border-top: 1px solid var(--header-border);
    padding: 1rem 1.5rem;
    font-size: 0.75rem;
    color: var(--text-tertiary);
  }

  footer .inner {
    display: flex;
    justify-content: space-between;
  }

  @media (width >= 48rem) {
    .clock,
    .desktop-wrap {
      display: flex;
    }

    .burger {
      display: none;
    }
  }

  @media (width < 38rem) {
    .bar {
      height: 3.25rem;
      padding-inline: 1rem;
    }

    .meta {
      gap: 0.15rem;
    }

    .meta .btn {
      min-width: 2.75rem;
      min-height: 2.75rem;
      padding: 0.5rem;
    }

    .scrim {
      inset-block-start: 3.25rem;
    }

    nav.mobile {
      inset-block-start: 3.25rem;
      width: min(19rem, 100%);
      height: calc(100vh - 3.25rem);
    }

    main {
      padding: 1rem;
    }

    footer {
      padding-inline: 1rem;
    }
  }
</style>
