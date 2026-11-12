<script lang="ts" module>
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
    chart: ['M3 3v18h18', 'M8 17v-5M13 17V8M18 17v-9'],
    user: ['M20 21a8 8 0 0 0-16 0', 'M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z'],
    spark: [
      'M12 3l1.9 5.6L19.5 10.5l-5.6 1.9L12 18l-1.9-5.6L4.5 10.5l5.6-1.9Z',
      'M19 17l.8 2.2L22 20l-2.2.8L19 23l-.8-2.2L16 20l2.2-.8Z',
    ],
    thought: [
      'M9 18h6M10 21h4',
      'M12 3a6 6 0 0 0-3.5 10.9c.6.5 1 1.2 1 2.1h5c0-.9.4-1.6 1-2.1A6 6 0 0 0 12 3Z',
    ],
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
    grid: ['M3 3h8v8H3zM13 3h8v8h-8zM3 13h8v8H3zM13 13h8v8h-8z'],
    rows: ['M3 5h18M3 12h18M3 19h18'],
    pulse: ['M2 12h4l3-8 6 16 3-8h4'],
    info: ['M12 11v6m0-10h.01', 'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z'],
    file: ['M14 3H6v18h12V7l-4-4Z', 'M14 3v4h4M9 12h6M9 16h6'],
    pencil: ['M4 20h4L19 9l-4-4L4 16v4Z', 'm13 7 4 4'],
    globe: [
      'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z',
      'M3 12h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9Z',
    ],
    branch: [
      'M6 3v12',
      'M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
      'M18 9c0 6-12 3-12 6',
    ],
    pin: ['M15 4.5l-4 4-4 1.5-1.5 1.5 7 7 1.5-1.5 1.5-4 4-4', 'M9 15l-4.5 4.5M14.5 4l5.5 5.5'],
    expand: ['M14 4h6v6M10 20H4v-6', 'm20 4-7 7M4 20l7-7'],
    lock: ['M5 11h14v10H5z', 'M8 11V7a4 4 0 0 1 8 0v4'],
    archive: ['M3 4h18v4H3z', 'M5 8v12h14V8M10 12h4'],
    lan: ['M9 3h6v5H9zM3 16h6v5H3zM15 16h6v5h-6z', 'M12 8v4M6 16v-4h12v4'],
    eye: [
      'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z',
      'M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
    ],
    'eye-off': [
      'm3 3 18 18',
      'M10.6 5.1Q11.3 5 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3.2 4.1M6.6 6.6C3.8 8.3 2 12 2 12s3.5 7 10 7a9.6 9.6 0 0 0 5.4-1.6',
      'M9.9 9.9a3 3 0 0 0 4.2 4.2',
    ],
    shrink: ['M4 14h6v6M20 10h-6V4', 'm4 20 6-6M20 4l-6 6'],
  };
  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
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
