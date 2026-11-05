<script lang="ts">
  const paths = {
    sun: [
      'M12 3V1m0 22v-2M3 12H1m22 0h-2M4.2 4.2l1.4 1.4m12.8 12.8 1.4 1.4M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4',
      'M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0Z',
    ],
    moon: ['M20.9 13.2A9 9 0 0 1 10.8 3.1a9 9 0 1 0 10.1 10.1Z'],
    monitor: ['M3 3h18v13H3z', 'M12 16v5m-4 0h8'],
    logs: ['M8 6h13M8 12h13M8 18h13', 'M3 6h.01M3 12h.01M3 18h.01'],
    sessions: ['M21 11a8 8 0 0 1-8 8H5l-4 3V11a8 8 0 0 1 8-8h4a8 8 0 0 1 8 8Z'],
    storage: [
      'M20 6c0 2-3.6 3-8 3S4 8 4 6s3.6-3 8-3 8 1 8 3Z',
      'M4 6v6c0 2 3.6 3 8 3s8-1 8-3V6M4 12v6c0 2 3.6 3 8 3s8-1 8-3v-6',
    ],
    search: ['M21 21l-5-5', 'M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z'],
    filter: ['M4 7h16M4 17h16', 'M8 4v6M16 14v6'],
    refresh: ['M20 7v5h-5M4 17v-5h5', 'M6.1 6.1A8 8 0 0 1 19.5 10M4.5 14a8 8 0 0 0 13.4 3.9'],
    chevron: ['m9 5 7 7-7 7'],
    close: ['m6 6 12 12M6 18 18 6'],
    check: ['m5 12 4 4L19 6'],
    alert: [
      'M12 8v5m0 3h.01',
      'm10.3 3.9-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3.1l-8-14a2 2 0 0 0-3.4 0Z',
    ],
    tool: [
      'm14 6 4 4',
      'M20.3 4.2a6 6 0 0 1-7.5 8.9l-7 7a2 2 0 0 1-2.9-2.9l7-7a6 6 0 0 1 8.9-7.5L15 6.5 17.5 9l2.8-4.8Z',
    ],
    code: ['m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18'],
    copy: ['M9 9h12v12H9z', 'M15 5V3H3v12h2'],
    arrow: ['M5 12h14m-5-5 5 5-5 5'],
    shield: ['m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z', 'm8 12 3 3 5-6'],
    terminal: ['m4 6 6 6-6 6m9 0h7'],
    info: ['M12 11v6m0-10h.01', 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z'],
  };
  let { name, size = 16 }: { name: keyof typeof paths; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  >{#each paths[name] as d}<path {d} />{/each}</svg
>
