<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto(link("/feed?view=discover"), { replaceState: true });
  });
</script>

<p class="redirect">Discover is now part of the <a href={link("/feed?view=discover")}>Feed</a>.</p>

<style>
  .redirect {
    color: var(--text-secondary);
    font-size: 0.8125rem;
  }
</style>
