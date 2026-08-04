<script lang="ts">
  let {
    active,
    mailCount,
  }: {
    active: "inbox" | "discover" | "mail" | "library" | "review";
    mailCount?: number;
  } = $props();
</script>

<nav aria-label="Feed view">
  <a class:active={active === "inbox"} href="/feed">Inbox</a>
  <a class:active={active === "discover"} href="/feed?view=discover">Discover</a>
  <a
    class:active={active === "mail"}
    href="/feed?view=mail"
    aria-label={mailCount === undefined ? "Mail proposals" : `Mail proposals, ${mailCount}`}
  >
    Mail
    {#if mailCount !== undefined}<span class="count mono">{mailCount}</span>{/if}
  </a>
  <a class:active={active === "library"} href="/feed/library">Library</a>
  <a class:active={active === "review"} href="/feed/review">Review</a>
</nav>

<style>
  nav {
    display: flex;
    gap: 1rem;
    margin: -0.35rem 0 1.25rem;
    border-bottom: 1px solid var(--card-border);
  }

  a {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin-bottom: -1px;
    padding: 0.55rem 0;
    border-bottom: 2px solid transparent;
    color: var(--text-tertiary);
    font-size: 0.8125rem;
    font-weight: 550;
    text-decoration: none;
  }

  a:hover {
    color: var(--text-secondary);
  }

  a.active {
    border-color: var(--primary);
    color: var(--primary);
  }

  .count {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  a.active .count {
    color: inherit;
  }
</style>
