<!--
  ============================================================================
  Field.svelte — Horizon Glass Form Field Wrapper
  ============================================================================
  Accessible form control layout wrapper providing standard label, helper
  hint, validation error indicators, and input container styling.

  Props:
    - label?: string — field label text
    - hint?: string — helper text displayed below control or label
    - error?: string — validation error text
    - id?: string — input id to associate with label
    - for?: string — alias for id
    - required?: boolean (default: false) — marks label with asterisk
    - disabled?: boolean (default: false) — applies disabled opacity
    - readOnly?: boolean (default: false)
    - horizontal?: boolean (default: false) — rows label and control side-by-side
    - class?: string (or className)
    - labelSnippet?: Snippet — custom label header
    - children?: Snippet — input / select / toggle control slot
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    label?: string;
    hint?: string;
    error?: string;
    id?: string;
    for?: string;
    required?: boolean;
    disabled?: boolean;
    readOnly?: boolean;
    horizontal?: boolean;
    class?: string;
    className?: string;
    labelSnippet?: Snippet;
    children?: Snippet;
    [key: string]: unknown;
  }

  let {
    label,
    hint,
    error,
    id,
    for: htmlFor,
    required = false,
    disabled = false,
    readOnly = false,
    horizontal = false,
    class: extraClass = '',
    className = '',
    labelSnippet,
    children,
    ...rest
  }: Props = $props();

  const targetId = $derived(id || htmlFor);
  const combinedClass = $derived([
    'field',
    horizontal ? 'field--horizontal' : '',
    disabled ? 'field--disabled' : '',
    readOnly ? 'field--readonly' : '',
    error ? 'field--has-error' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div class={combinedClass} {...rest}>
  {#if labelSnippet}
    <div class="field__label-row">
      {@render labelSnippet()}
      {#if required}
        <span class="field__required" aria-hidden="true">*</span>
      {/if}
    </div>
  {:else if label}
    <div class="field__label-row">
      {#if targetId}
        <label for={targetId} class="field__label">
          {label}
          {#if required}
            <span class="field__required" aria-hidden="true">*</span>
          {/if}
        </label>
      {:else}
        <span class="field__label">
          {label}
          {#if required}
            <span class="field__required" aria-hidden="true">*</span>
          {/if}
        </span>
      {/if}
    </div>
  {/if}

  <div class="field__control">
    {@render children?.()}
  </div>

  {#if error}
    <div class="field__error" role="alert">
      <svg class="field__error-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      <span>{error}</span>
    </div>
  {:else if hint}
    <div class="field__hint">
      {hint}
    </div>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
    width: 100%;
  }

  .field--horizontal {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4, 16px);
  }

  .field--disabled {
    opacity: 0.5;
    pointer-events: none;
  }

  .field__label-row {
    display: flex;
    align-items: center;
    gap: var(--space-1, 4px);
  }

  .field__label {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    line-height: var(--leading-tight, 1.2);
    color: var(--text, #e8ecf4);
    letter-spacing: 0.02em;
  }

  .field__required {
    color: var(--accent-red, #ef4444);
    font-weight: 700;
  }

  .field__control {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
  }

  /* Deep style common native controls placed inside field */
  .field__control :global(input:not([type='checkbox']):not([type='radio'])),
  .field__control :global(select),
  .field__control :global(textarea) {
    width: 100%;
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    color: var(--text, #e8ecf4);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-family: var(--font-body, inherit);
    font-size: var(--text-sm, 0.875rem);
    line-height: var(--leading-snug, 1.35);
    outline: none;
    transition:
      border-color var(--dur-fast, 120ms) var(--ease-out-expo),
      box-shadow var(--dur-fast, 120ms) var(--ease-out-expo),
      background-color var(--dur-fast, 120ms) var(--ease-out-expo);
  }

  .field__control :global(input:focus),
  .field__control :global(select:focus),
  .field__control :global(textarea:focus) {
    border-color: var(--border-focus, rgba(16, 185, 129, 0.6));
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  .field--has-error .field__control :global(input),
  .field--has-error .field__control :global(select),
  .field--has-error .field__control :global(textarea) {
    border-color: var(--accent-red, #ef4444);
  }

  .field__hint {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    line-height: var(--leading-snug, 1.35);
  }

  .field__error {
    display: flex;
    align-items: center;
    gap: var(--space-1, 4px);
    font-size: var(--text-xs, 0.75rem);
    color: var(--accent-red, #ef4444);
    font-weight: 500;
    line-height: var(--leading-snug, 1.35);
  }

  .field__error-icon {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }
</style>
