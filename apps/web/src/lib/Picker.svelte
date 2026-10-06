<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import Check from '@lucide/svelte/icons/check';
  import Modal from './Modal.svelte';
  let {
    label,
    value,
    options,
    onchange,
    disabled = false,
    class: className = '',
  }: {
    label: string;
    value: string;
    options: { value: string; label: string; detail?: string }[];
    onchange: (value: string) => void;
    disabled?: boolean;
    class?: string;
  } = $props();
  let open = $state(false);
  let cursor = $state(0);
  let list: HTMLDivElement;
  const id = $props.id();
  function show() {
    cursor = Math.max(
      0,
      options.findIndex((item) => item.value === value),
    );
    open = true;
  }
  function keydown(event: KeyboardEvent) {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    cursor =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? options.length - 1
          : (cursor + (event.key === 'ArrowUp' ? -1 : 1) + options.length) % options.length;
    list.querySelectorAll<HTMLButtonElement>('[role="option"]')[cursor]?.focus();
  }
</script>

<button
  type="button"
  class={`picker-trigger ${className}`}
  {disabled}
  role="combobox"
  aria-label={label}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-controls={id}
  data-value={value}
  onclick={show}
  onkeydown={(event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      show();
    }
  }}
>
  <span>{options.find((item) => item.value === value)?.label ?? value}</span><ChevronDown
    size={15}
  />
</button>
<Modal {open} title={label} closeLabel="Fechar" onclose={() => (open = false)}>
  <div
    class="picker-options"
    {id}
    role="listbox"
    aria-label={label}
    tabindex="-1"
    bind:this={list}
    onkeydown={keydown}
  >
    {#each options as item, index (item.value)}
      <button
        type="button"
        role="option"
        data-value={item.value}
        aria-label={item.label}
        aria-selected={item.value === value}
        data-dialog-focus={index === cursor ? '' : undefined}
        tabindex={index === cursor ? 0 : -1}
        onfocus={() => (cursor = index)}
        onclick={() => {
          onchange(item.value);
          open = false;
        }}
      >
        <span class="grow"
          ><strong>{item.label}</strong>{#if item.detail}<small>{item.detail}</small>{/if}</span
        >
        {#if item.value === value}<Check size={18} aria-hidden="true" />{/if}
      </button>
    {/each}
  </div>
</Modal>
