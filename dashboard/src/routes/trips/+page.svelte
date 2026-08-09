<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto(link("/travel"), { replaceState: true });
  });
</script>

<p>Travel planning has moved to <a href={link("/travel")}>Travel</a>.</p>
