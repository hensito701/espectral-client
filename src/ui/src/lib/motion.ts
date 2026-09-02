/* ==========================================================================
   Espectral Horizon Glass — Motion & Transition System
   TypeScript helper module mirroring CSS motion tokens.
   Safe to import in .svelte, .ts, and .svelte.ts files.
   ========================================================================== */

import type { TransitionConfig } from 'svelte/transition';

/**
 * Standard duration tokens in milliseconds, matching CSS --dur-* variables.
 */
export const DURATION = {
  fast: 120,
  med: 260,
  slow: 450,
} as const;

export type DurationKey = keyof typeof DURATION;

/**
 * Bezier curve control points and CSS string equivalents matching tokens.css.
 */
export const EASING = {
  // Cubic bezier parameter arrays [x1, y1, x2, y2]
  expoOutPoints: [0.16, 1, 0.3, 1] as const,
  springPoints: [0.34, 1.56, 0.64, 1] as const,
  springSoftPoints: [0.2, 0.8, 0.2, 1] as const,

  // CSS cubic-bezier strings
  expo: 'cubic-bezier(0.16, 1, 0.3, 1)',
  spring: 'cubic-bezier(0.34, 1.56, 0.64, 1)',
  springSoft: 'cubic-bezier(0.2, 0.8, 0.2, 1)',

  /**
   * Easing function for Svelte transitions: Exponential Out curve.
   */
  expoOut(t: number): number {
    return t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
  },

  /**
   * Easing function for Svelte transitions: Soft spring / fluid decelerate.
   */
  springSoftFn(t: number): number {
    const ts = t * t;
    const tc = ts * t;
    return 6 * tc * ts - 15 * ts * ts + 10 * tc;
  },

  /**
   * Easing function for Svelte transitions: Bouncy overshoot spring.
   */
  springBounce(t: number): number {
    const c4 = (2 * Math.PI) / 3;
    return t === 0
      ? 0
      : t === 1
        ? 1
        : Math.pow(2, -10 * t) * Math.sin((t * 10 - 0.75) * c4) + 1;
  },
} as const;

/**
 * Returns true if the user's operating system prefers reduced motion.
 * Safe to call in browser environments; returns false on SSR.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}
export function onReducedMotionChange(callback: (reduced: boolean) => void): () => void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return () => {};
  }
  const mediaQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
  const listener = (event: MediaQueryListEvent) => {
    callback(event.matches);
  };

  mediaQuery.addEventListener('change', listener);
  return () => mediaQuery.removeEventListener('change', listener);
}

/* ==========================================================================
   Svelte Transition Custom Factories
   Use with Svelte transition: / in: / out: directives.
   ========================================================================== */

export interface FlyYParams {
  y?: number;
  duration?: number;
  delay?: number;
  easing?: (t: number) => number;
  opacity?: number;
}

/**
 * Vertical fly transition with opacity fade and expo deceleration.
 * Automatically respects prefers-reduced-motion by omitting the translate.
 */
export function flyY(
  node: Element,
  {
    y = 12,
    duration = DURATION.med,
    delay = 0,
    easing = EASING.expoOut,
    opacity = 0,
  }: FlyYParams = {},
): TransitionConfig {
  const reduced = prefersReducedMotion();
  const effectiveDuration = reduced ? 0 : duration;
  const effectiveY = reduced ? 0 : y;

  const style = getComputedStyle(node);
  const targetOpacity = +style.opacity;
  const transform = style.transform === 'none' ? '' : style.transform;
  const od = targetOpacity * (1 - opacity);

  return {
    delay,
    duration: effectiveDuration,
    easing,
    css: (_t, u) => `
      transform: ${transform} translateY(${u * effectiveY}px);
      opacity: ${targetOpacity - od * u};
    `,
  };
}

export interface FadeParams {
  duration?: number;
  delay?: number;
  easing?: (t: number) => number;
}

/**
 * Smooth opacity fade transition.
 */
export function fade(
  node: Element,
  {
    duration = DURATION.fast,
    delay = 0,
    easing = EASING.expoOut,
  }: FadeParams = {},
): TransitionConfig {
  const reduced = prefersReducedMotion();
  const effectiveDuration = reduced ? 0 : duration;
  const style = getComputedStyle(node);
  const targetOpacity = +style.opacity;

  return {
    delay,
    duration: effectiveDuration,
    easing,
    css: (t) => `opacity: ${t * targetOpacity};`,
  };
}

export interface ScalePopParams {
  start?: number;
  duration?: number;
  delay?: number;
  easing?: (t: number) => number;
  opacity?: number;
}

/**
 * Scale pop-in transition for modals, cards, and command palettes.
 */
export function scalePop(
  node: Element,
  {
    start = 0.94,
    duration = DURATION.med,
    delay = 0,
    easing = EASING.springSoftFn,
    opacity = 0,
  }: ScalePopParams = {},
): TransitionConfig {
  const reduced = prefersReducedMotion();
  const effectiveDuration = reduced ? 0 : duration;
  const effectiveScale = reduced ? 1 : start;

  const style = getComputedStyle(node);
  const targetOpacity = +style.opacity;
  const transform = style.transform === 'none' ? '' : style.transform;
  const sd = 1 - effectiveScale;
  const od = targetOpacity * (1 - opacity);

  return {
    delay,
    duration: effectiveDuration,
    easing,
    css: (_t, u) => `
      transform: ${transform} scale(${1 - sd * u});
      opacity: ${targetOpacity - od * u};
    `,
  };
}

export interface SlideDrawerParams {
  y?: number;
  duration?: number;
  delay?: number;
  easing?: (t: number) => number;
}

/**
 * Bottom drawer slide-up transition for the PiP flight deck.
 */
export function slideDrawer(
  node: Element,
  {
    y = 260,
    duration = DURATION.med,
    delay = 0,
    easing = EASING.expoOut,
  }: SlideDrawerParams = {},
): TransitionConfig {
  const reduced = prefersReducedMotion();
  const effectiveDuration = reduced ? 0 : duration;
  const effectiveY = reduced ? 0 : y;

  const style = getComputedStyle(node);
  const transform = style.transform === 'none' ? '' : style.transform;

  return {
    delay,
    duration: effectiveDuration,
    easing,
    css: (_t, u) => `
      transform: ${transform} translateY(${u * effectiveY}px);
    `,
  };
}
