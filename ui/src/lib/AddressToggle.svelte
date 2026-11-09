<!--
  Shows or masks the gateway's address wherever it appears (`addressView`).
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import { addressView } from './address.svelte.ts';
</script>

<button
  class="icon-button address-toggle"
  aria-pressed={addressView.revealed}
  title={addressView.revealed ? 'Mask the address' : 'Show the address'}
  aria-label={addressView.revealed ? 'Mask the address' : 'Show the address'}
  onclick={() => (addressView.revealed = !addressView.revealed)}
  ><Icon name={addressView.revealed ? 'eye-off' : 'eye'} size={13} /></button
>
