<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto("/travel", { replaceState: true });
  });
</script>

<p>Travel planning has moved to <a href="/travel">Travel</a>.</p>
