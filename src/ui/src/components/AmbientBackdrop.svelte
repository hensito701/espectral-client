<script lang="ts">
  /* ==========================================================================
     AmbientBackdrop — The Horizon Glass Dynamic Ambient Stage
     Layer 0 (z-index: 0): Transform-only GPU drift of radial ambient lights,
     faint subtle grid, edge vignette, and --ambient-hue integration.
     C13: while any liveLaunch is running, pause orb animations via
     animation-play-state to reduce GPU contention during boot.
     ========================================================================== */
  import { liveLaunches } from '../lib/stores';

  const isPaused = $derived.by(() => {
    const v = $liveLaunches.value;
    return Array.isArray(v) && v.some((l) => l.running);
  });
</script>

<div class="ambient-stage" class:ambient-stage--paused={isPaused} aria-hidden="true">
  <!-- Glowing drifting ambient orbs -->
  <div class="ambient-orb ambient-orb--primary" class:ambient-orb--paused={isPaused}></div>
  <div class="ambient-orb ambient-orb--secondary" class:ambient-orb--paused={isPaused}></div>
  <div class="ambient-orb ambient-orb--accent" class:ambient-orb--paused={isPaused}></div>

  <!-- Faint tech grid overlay -->
  <div class="ambient-grid"></div>

  <!-- Edge vignette -->
  <div class="ambient-vignette"></div>
</div>

<style>
  .ambient-stage {
    position: fixed;
    inset: 0;
    z-index: var(--z-ambient, 0);
    overflow: hidden;
    pointer-events: none;
    user-select: none;
    contain: strict;
  }

  /* Orbs base */
  .ambient-orb {
    position: absolute;
    border-radius: var(--radius-pill, 9999px);
    filter: blur(var(--glass-blur-strong, 80px));
    will-change: transform;
    opacity: 0.85;
    mix-blend-mode: screen;
  }

  [data-theme='light'] .ambient-orb {
    mix-blend-mode: multiply;
    opacity: 0.45;
  }

  /* Primary orb (top-left to center drift) */
  .ambient-orb--primary {
    top: -10vw;
    left: -10vw;
    width: 55vw;
    height: 55vw;
    max-width: 800px;
    max-height: 800px;
    background: radial-gradient(
      circle at center,
      var(--ambient-1, rgba(16, 185, 129, 0.22)) 0%,
      hsla(var(--ambient-hue, 160), 80%, 45%, 0.12) 40%,
      transparent 70%
    );
    animation: drift-primary 28s ease-in-out infinite alternate;
  }

  /* Secondary orb (bottom-right to center drift) */
  .ambient-orb--secondary {
    bottom: -15vw;
    right: -10vw;
    width: 60vw;
    height: 60vw;
    max-width: 900px;
    max-height: 900px;
    background: radial-gradient(
      circle at center,
      var(--ambient-2, rgba(6, 182, 212, 0.18)) 0%,
      hsla(calc(var(--ambient-hue, 160) + 40), 75%, 50%, 0.08) 45%,
      transparent 70%
    );
    animation: drift-secondary 36s ease-in-out infinite alternate;
  }

  /* Accent orb (mid-screen subtle pulse) */
  .ambient-orb--accent {
    top: 20%;
    right: 25%;
    width: 40vw;
    height: 40vw;
    max-width: 600px;
    max-height: 600px;
    background: radial-gradient(
      circle at center,
      hsla(var(--ambient-hue, 160), 90%, 55%, 0.1) 0%,
      rgba(255, 215, 0, 0.04) 50%,
      transparent 70%
    );
    animation: drift-accent 42s ease-in-out infinite alternate;
  }

  /* Faint subtle grid overlay */
  .ambient-grid {
    position: absolute;
    inset: 0;
    background-image:
      linear-gradient(to right, rgba(255, 255, 255, 0.018) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(255, 255, 255, 0.018) 1px, transparent 1px);
    background-size: 32px 32px;
    mask-image: radial-gradient(circle at 50% 50%, black 30%, transparent 80%);
    -webkit-mask-image: radial-gradient(circle at 50% 50%, black 30%, transparent 80%);
  }

  [data-theme='light'] .ambient-grid {
    background-image:
      linear-gradient(to right, rgba(0, 0, 0, 0.025) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(0, 0, 0, 0.025) 1px, transparent 1px);
  }

  /* Vignette */
  .ambient-vignette {
    position: absolute;
    inset: 0;
    background: radial-gradient(
      ellipse at center,
      transparent 50%,
      rgba(6, 10, 20, 0.6) 100%
    );
  }

  [data-theme='light'] .ambient-vignette {
    background: radial-gradient(
      ellipse at center,
      transparent 50%,
      rgba(244, 246, 251, 0.45) 100%
    );
  }

  /* Keyframe Animations (strictly transform for 60-120fps GPU performance) */
  @keyframes drift-primary {
    0% {
      transform: translate3d(0, 0, 0) scale(1);
    }
    50% {
      transform: translate3d(8vw, 6vh, 0) scale(1.1);
    }
    100% {
      transform: translate3d(4vw, 12vh, 0) scale(0.95);
    }
  }

  @keyframes drift-secondary {
    0% {
      transform: translate3d(0, 0, 0) scale(1);
    }
    50% {
      transform: translate3d(-10vw, -8vh, 0) scale(1.08);
    }
    100% {
      transform: translate3d(-5vw, -4vh, 0) scale(0.92);
    }
  }

  @keyframes drift-accent {
    0% {
      transform: translate3d(0, 0, 0) rotate(0deg);
    }
    50% {
      transform: translate3d(-6vw, 8vh, 0) rotate(90deg);
    }
    100% {
      transform: translate3d(5vw, -6vh, 0) rotate(180deg);
    }
  }

  /* Accessibility: Prefers-reduced-motion */
  @media (prefers-reduced-motion: reduce) {
    .ambient-orb {
      animation: none !important;
      transform: none !important;
    }
  }

  /* C13: pause orb drift while a launch is running */
  .ambient-orb--paused {
    animation-play-state: paused !important;
  }
</style>
