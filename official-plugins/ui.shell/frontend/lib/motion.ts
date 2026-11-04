import { cubicOut } from 'svelte/easing';
import { fly, type TransitionConfig } from 'svelte/transition';

const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
const easing = 'cubic-bezier(.333333,1,.666667,1)';

// Match Launcher.UI/Motion/FadeSlideTransition.cs, including its shorter list preset.
export function fadeSlide(node: Element, { direction = 1, compact = false } = {}, { direction: phase }: { direction: 'in' | 'out' | 'both' }): TransitionConfig {
  const exit = phase === 'out';
  return fly(node, {
    x: direction * (exit ? (compact ? -6 : -12) : (compact ? 12 : 24)),
    duration: reduced() ? 0 : exit ? (compact ? 120 : 160) : (compact ? 200 : 240),
    easing: cubicOut,
  });
}

export function entrance(node: Element, { delay = 0 } = {}): TransitionConfig {
  return fly(node, { y: 10, delay: reduced() ? 0 : delay, duration: reduced() ? 0 : 300, easing: cubicOut });
}

// Text changes replace the previous effect; frequent percentage updates stay readable.
export function animateText(node: HTMLElement, value: unknown) {
  let previous = value;
  let changedAt = -Infinity;
  let animation: Animation | undefined;
  return {
    update(next: unknown) {
      if (Object.is(previous, next)) return;
      previous = next;
      const now = performance.now();
      const quiet = now - changedAt >= 250;
      changedAt = now;
      animation?.cancel();
      if (quiet && !reduced()) {
        animation = node.animate([{ opacity: .45, translate: '0 3px' }, { opacity: 1, translate: '0 0' }], { duration: 140, easing });
      }
    },
    destroy() { animation?.cancel(); },
  };
}
