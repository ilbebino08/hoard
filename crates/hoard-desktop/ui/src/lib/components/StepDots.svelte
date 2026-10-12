<script lang="ts">
  /**
   * Where you are in a sequence: the welcome wizard and the guided tour. Rings,
   * not fills, in dark green; the current one stretches and lights up a little.
   * Black inside, so the tour's, which float over the page, stay readable.
   */
  type Props = {
    count: number;
    current: number;
    label?: string;
  };
  let { count, current, label }: Props = $props();
</script>

<div
  class="flex items-center justify-center gap-2"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  {#each { length: count } as _, i (i)}
    <span
      class="h-2 rounded-full border bg-black transition-all duration-300 {i === current
        ? 'w-6 border-emerald-500'
        : i < current
          ? 'w-2 border-emerald-700'
          : 'w-2 border-emerald-900'}"
    ></span>
  {/each}
</div>
