<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';

  type Preference = 'light' | 'dark' | 'system';
  const options = [
    { value: 'light', icon: 'sun', label: 'Light theme' },
    { value: 'dark', icon: 'moon', label: 'Dark theme' },
    { value: 'system', icon: 'monitor', label: 'System theme' },
  ] as const;
  const parse = (value: string | null | undefined): Preference =>
    value === 'light' || value === 'dark' ? value : 'system';
  let preference = $state<Preference>('system');

  const index = $derived(options.findIndex((option) => option.value === preference));
  const current = $derived(options[index]);
  const next = $derived(options[(index + 1) % options.length]);
  const label = $derived(`${current.label}. Switch to ${next.label.toLowerCase()}.`);

  function apply() {
    document.documentElement.dataset.themePreference = preference;
    document.documentElement.dataset.theme =
      preference === 'system'
        ? matchMedia('(prefers-color-scheme: dark)').matches
          ? 'dark'
          : 'light'
        : preference;
  }

  function choose(value: Preference) {
    preference = value;
    apply();
    try {
      localStorage.setItem('tgv.theme', value);
    } catch {
      // The choice still works for this page when persistence is unavailable.
    }
  }

  onMount(() => {
    preference = parse(document.documentElement.dataset.themePreference);
    const media = matchMedia('(prefers-color-scheme: dark)');
    const sync = (event: StorageEvent) => {
      if (event.key === 'tgv.theme' || event.key === null) {
        preference = parse(event.newValue);
        apply();
      }
    };
    apply();
    media.addEventListener('change', apply);
    window.addEventListener('storage', sync);
    return () => {
      media.removeEventListener('change', apply);
      window.removeEventListener('storage', sync);
    };
  });
</script>

<div class="theme-control">
  <button
    class="theme-button"
    type="button"
    aria-label={label}
    title={label}
    onclick={() => choose(next.value)}><Icon name={current.icon} size={15} /></button
  >
</div>
