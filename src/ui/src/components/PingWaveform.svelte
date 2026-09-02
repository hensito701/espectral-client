<!--
  ============================================================================
  PingWaveform.svelte — Horizon Glass Deterministic Ping Sparkline
  ============================================================================
  Pure SVG sparkline component for server ping telemetry.
  Generates a deterministic pseudo-waveform seeded by ping latency,
  with emerald/amber/red color-coding, gradient fill, and transform-only
  motion transitions (disabled under prefers-reduced-motion).

  Props:
    - pingMs?: number | null (latency in milliseconds; null = offline/unknown)
    - online?: boolean (default: true)
    - height?: number (default: 24)
    - width?: number (default: 84)
    - showLabel?: boolean (default: false) — renders 'XX ms' text next to waveform
    - class?: string (or className)
-->
<script lang="ts">
  interface Props {
    pingMs?: number | null;
    online?: boolean;
    height?: number;
    width?: number;
    showLabel?: boolean;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    pingMs = null,
    online = true,
    height = 24,
    width = 84,
    showLabel = false,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  // Color tiers based on latency & connectivity
  const statusColor = $derived.by(() => {
    if (!online || pingMs === null || pingMs === undefined) {
      return {
        stroke: 'var(--accent-red, #ef4444)',
        fill: 'rgba(239, 68, 68, 0.15)',
        glow: 'rgba(239, 68, 68, 0.3)',
        tier: 'offline',
      };
    }
    if (pingMs <= 65) {
      return {
        stroke: 'var(--accent, #10b981)',
        fill: 'rgba(16, 185, 129, 0.18)',
        glow: 'rgba(16, 185, 129, 0.4)',
        tier: 'good',
      };
    }
    if (pingMs <= 140) {
      return {
        stroke: 'var(--accent-alt, #f59e0b)',
        fill: 'rgba(245, 158, 11, 0.18)',
        glow: 'rgba(245, 158, 11, 0.4)',
        tier: 'warn',
      };
    }
    return {
      stroke: 'var(--accent-red, #ef4444)',
      fill: 'rgba(239, 68, 68, 0.18)',
      glow: 'rgba(239, 68, 68, 0.4)',
      tier: 'bad',
    };
  });

  // Unique gradient ID per component instance
  const gradId = $derived(`ping-grad-${Math.abs((pingMs || 0) * 31 + (online ? 1 : 0)) % 10000}`);

  // Deterministic seeded wave generation
  const waveData = $derived.by(() => {
    const pointCount = 7;
    const paddingX = 4;
    const paddingY = 4;
    const innerW = width - paddingX * 2;
    const innerH = height - paddingY * 2;

    if (!online || pingMs === null || pingMs === undefined) {
      // Offline: flat bottom dashed line
      const y = height - paddingY;
      const pts: Array<[number, number]> = [];
      for (let i = 0; i < pointCount; i++) {
        const x = paddingX + (innerW / (pointCount - 1)) * i;
        pts.push([x, y]);
      }
      const pathD = pts.map((p, idx) => (idx === 0 ? `M ${p[0]} ${p[1]}` : `L ${p[0]} ${p[1]}`)).join(' ');
      return {
        pathD,
        areaD: `${pathD} L ${width - paddingX} ${height} L ${paddingX} ${height} Z`,
        points: pts,
        lastPoint: pts[pts.length - 1],
      };
    }

    // Seeded generator
    let seed = Math.abs(Math.floor(pingMs * 137 + 49297));
    const nextRandom = () => {
      seed = (seed * 9301 + 49297) % 233280;
      return seed / 233280;
    };

    const pts: Array<[number, number]> = [];
    const baseVariance = Math.min(Math.max(pingMs * 0.12, 2), innerH * 0.4);

    for (let i = 0; i < pointCount; i++) {
      const x = paddingX + (innerW / (pointCount - 1)) * i;
      // Latency jitter curve: recent points settle near true ping relative height
      const jitter = (nextRandom() - 0.5) * baseVariance;
      // Normal latency baseline in top 20%-70% of box
      const normY = (innerH * 0.5) + jitter;
      const clampedY = Math.max(paddingY, Math.min(height - paddingY, normY));
      pts.push([x, clampedY]);
    }

    // Smooth spline-like path using cubic beziers
    let pathD = `M ${pts[0][0]} ${pts[0][1]}`;
    for (let i = 0; i < pts.length - 1; i++) {
      const curr = pts[i];
      const next = pts[i + 1];
      const cpX = (curr[0] + next[0]) / 2;
      pathD += ` C ${cpX} ${curr[1]}, ${cpX} ${next[1]}, ${next[0]} ${next[1]}`;
    }

    const areaD = `${pathD} L ${width - paddingX} ${height} L ${paddingX} ${height} Z`;

    return {
      pathD,
      areaD,
      points: pts,
      lastPoint: pts[pts.length - 1],
    };
  });

  const combinedClass = $derived([
    'ping-waveform',
    `ping-waveform--${statusColor.tier}`,
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div class={combinedClass} {...rest} aria-label={online && pingMs ? `${pingMs} ms` : 'Offline'}>
  <svg
    {width}
    {height}
    viewBox={`0 0 ${width} ${height}`}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    class="ping-svg"
  >
    <defs>
      <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color={statusColor.stroke} stop-opacity="0.35" />
        <stop offset="100%" stop-color={statusColor.stroke} stop-opacity="0.0" />
      </linearGradient>
    </defs>

    <!-- Area Fill -->
    <path d={waveData.areaD} fill={`url(#${gradId})`} class="wave-area" />

    <!-- Sparkline Stroke -->
    <path
      d={waveData.pathD}
      stroke={statusColor.stroke}
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      stroke-dasharray={online ? 'none' : '3 3'}
      class="wave-line"
    />

    <!-- Leading Beacon Dot (Latest Telemetry Probe) -->
    {#if waveData.lastPoint}
      <circle
        cx={waveData.lastPoint[0]}
        cy={waveData.lastPoint[1]}
        r="2.5"
        fill={statusColor.stroke}
        class="wave-dot"
      />
      {#if online}
        <circle
          cx={waveData.lastPoint[0]}
          cy={waveData.lastPoint[1]}
          r="4.5"
          fill="none"
          stroke={statusColor.stroke}
          stroke-width="1"
          opacity="0.4"
          class="wave-dot-ring"
        />
      {/if}
    {/if}
  </svg>

  {#if showLabel}
    <span class="ping-label font-pixel">
      {#if online && pingMs !== null && pingMs !== undefined}
        {pingMs}ms
      {:else if online}
        --ms
      {:else}
        OFF
      {/if}
    </span>
  {/if}
</div>

<style>
  .ping-waveform {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    user-select: none;
    vertical-align: middle;
  }

  .ping-svg {
    display: block;
    overflow: visible;
  }

  .wave-line {
    filter: drop-shadow(0 0 4px var(--wave-glow, rgba(16, 185, 129, 0.4)));
    transition: stroke var(--dur-med, 200ms) var(--ease-out-expo, ease-out);
  }

  .wave-dot-ring {
    animation: beaconPulse 2.4s cubic-bezier(0.16, 1, 0.3, 1) infinite;
    transform-origin: center;
  }

  .ping-label {
    font-size: var(--text-xs, 0.75rem);
    line-height: 1;
    color: var(--muted-strong, #b3c5e3);
    letter-spacing: -0.02em;
  }

  .ping-waveform--good {
    --wave-glow: rgba(16, 185, 129, 0.5);
  }

  .ping-waveform--warn {
    --wave-glow: rgba(245, 158, 11, 0.5);
  }

  .ping-waveform--bad,
  .ping-waveform--offline {
    --wave-glow: rgba(239, 68, 68, 0.4);
  }

  @keyframes beaconPulse {
    0% {
      r: 2.5;
      opacity: 0.8;
    }
    60% {
      r: 6.5;
      opacity: 0;
    }
    100% {
      r: 6.5;
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .wave-dot-ring {
      animation: none;
      opacity: 0.2;
    }
    .wave-line {
      transition: none;
    }
  }
</style>
