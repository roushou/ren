/**
 * Whether gateway addresses show in full. They start masked on every page
 * load, so a screenshot or a shared screen never shows them by accident;
 * revealing applies to every place that shows the address until the next
 * load. Copy buttons always copy the real address.
 */
export const addressView = $state({ revealed: false });
