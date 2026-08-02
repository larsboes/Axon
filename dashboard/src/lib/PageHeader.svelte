<script lang="ts">
  let { badge, title, desc }: { badge: string; title: string; desc?: string } = $props();
</script>

<header>
  <p class="badge">{badge}</p>
  <h1>{title}</h1>
  {#if desc}<p class="desc">{desc}</p>{/if}
</header>

<style>
  header {
    margin-bottom: 1.5rem;
    animation: fade-up 0.3s ease-out both;
  }

  .badge {
    margin: 0;
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--primary);
  }

  h1 {
    margin: 0.25rem 0 0;
    font-size: 1.875rem;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .desc {
    margin: 0.4rem 0 0;
    max-width: 36rem;
    font-size: 0.875rem;
    color: var(--text-secondary);
  }
</style>
