<script lang="ts">
  import type { Snippet } from 'svelte';
  let {
    open,
    title,
    closeLabel,
    onclose,
    children,
  }: { open: boolean; title: string; closeLabel: string; onclose: () => void; children: Snippet } =
    $props();
  let dialog: HTMLDialogElement;
  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  });
</script>

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="dialog-title">
  <div class="dialog-heading">
    <h2 id="dialog-title">{title}</h2>
    <button class="icon-button" aria-label={closeLabel} onclick={onclose}>×</button>
  </div>
  {@render children()}
</dialog>
