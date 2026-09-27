<!-- 应用 logo:与打包图标同款设计的矢量版(放大镜 + 文本行 + 星光)。 -->
<script lang="ts">
  let { size = 26 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 48 48" fill="none" aria-hidden="true">
  <defs>
    <linearGradient id="lg-accent" x1="10" y1="38" x2="40" y2="8" gradientUnits="userSpaceOnUse">
      <stop stop-color="var(--accent)" />
      <stop offset="1" stop-color="color-mix(in srgb, var(--accent) 55%, #22d3ee)" />
    </linearGradient>
  </defs>
  <!-- 镜柄 -->
  <path
    d="M28.2 28.2 38 38"
    stroke="url(#lg-accent)"
    stroke-width="5.4"
    stroke-linecap="round"
  />
  <!-- 镜圈 -->
  <circle cx="21.5" cy="21.5" r="12.5" stroke="url(#lg-accent)" stroke-width="5" />
  <!-- 文本行 -->
  <path d="M15 16.8h13" stroke="currentColor" stroke-width="3" stroke-linecap="round" opacity="0.92" />
  <path d="M15 22h9.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" opacity="0.55" />
  <path d="M15 27.2h11" stroke="currentColor" stroke-width="3" stroke-linecap="round" opacity="0.55" />
  <!-- 星光 -->
  <path
    d="M40.5 7.5 41.7 10.3 44.5 11.5 41.7 12.7 40.5 15.5 39.3 12.7 36.5 11.5 39.3 10.3Z"
    fill="currentColor"
    opacity="0.9"
  />
</svg>
