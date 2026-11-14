import type { Action } from 'svelte/action';

type Options = { disabled: boolean; move: (id: string, target: string, after: boolean) => Promise<unknown> };
type Pointer = { id: number; row: HTMLElement; x: number; y: number };

// Use pointer events so sorting can coexist with Tauri's native file drops.
export const reorder: Action<HTMLElement, Options> = (list, options) => {
  const scroll = list.closest<HTMLElement>('.scroll-pane') ?? list;
  const rows = () => Array.from(list.querySelectorAll<HTMLElement>('[data-reorder-id]'));
  let pointer: Pointer | undefined;
  let drag: ReturnType<typeof lift> | undefined;
  let frame = 0;
  let suppressClick = false;

  function lift(press: Pointer) {
    const entries = rows().map(row => ({ row, bounds: row.getBoundingClientRect() }));
    const source = entries.findIndex(entry => entry.row === press.row);
    const { bounds } = entries[source];
    const neighbor = entries[source + 1] ?? entries[source - 1];
    const gap = source + 1 < entries.length ? neighbor.bounds.top - bounds.bottom : bounds.top - neighbor.bounds.bottom;
    const preview = press.row.cloneNode(true) as HTMLElement;
    preview.removeAttribute('data-reorder-id');
    preview.setAttribute('data-reorder-preview', '');
    preview.setAttribute('aria-hidden', 'true');
    preview.inert = true;
    preview.popover = 'manual';
    preview.style.width = `${bounds.width}px`;
    preview.style.height = `${bounds.height}px`;
    // A top-layer preview follows the pointer outside the scrolling list and inherits the current theme.
    list.append(preview);
    preview.showPopover();
    press.row.setAttribute('data-reorder-source', '');
    list.setAttribute('data-reordering', '');
    return { entries, source, bounds, step: bounds.height + gap, preview,
      scrollTop: scroll.scrollTop, startX: press.x, startY: press.y, target: source };
  }

  function reset() {
    cancelAnimationFrame(frame);
    if (pointer && list.hasPointerCapture(pointer.id)) list.releasePointerCapture(pointer.id);
    if (drag) {
      drag.preview.remove();
      drag.entries[drag.source].row.removeAttribute('data-reorder-source');
      for (const { row } of drag.entries) row.style.removeProperty('translate');
      list.removeAttribute('data-reordering');
    }
    pointer = drag = undefined;
  }

  function place(x: number, y: number) {
    if (!drag) return;
    const { bounds, startX, startY, source, entries, step } = drag;
    drag.preview.style.transform = `translate(${bounds.left + x - startX}px, ${bounds.top + y - startY}px)`;
    const center = bounds.top + bounds.height / 2 + y - startY + scroll.scrollTop - drag.scrollTop;
    const before = entries.findIndex(({ bounds }) => center < bounds.top + bounds.height / 2);
    const slot = before < 0 ? entries.length : before;
    const target = slot - Number(slot > source);
    if (target === drag.target) return;
    drag.target = target;
    entries.forEach(({ row }, index) => {
      const shift = index > source && index <= target ? -step : index >= target && index < source ? step : 0;
      row.style.translate = shift ? `0 ${shift}px` : '';
    });
  }

  function scrollAtEdge() {
    if (!pointer || !drag) return;
    const { x, y } = pointer;
    const bounds = scroll.getBoundingClientRect();
    const edge = 28;
    const delta = y >= bounds.top - edge && y < bounds.top + edge ? -8
      : y <= bounds.bottom + edge && y > bounds.bottom - edge ? 8 : 0;
    if (delta) {
      const top = scroll.scrollTop;
      scroll.scrollTop += delta;
      if (scroll.scrollTop !== top) place(x, y);
    }
    frame = requestAnimationFrame(scrollAtEdge);
  }

  function down(event: PointerEvent) {
    suppressClick = false;
    if (drag || options.disabled || event.pointerType !== 'mouse' || event.button !== 0) return;
    const handle = (event.target as Element).closest('[data-reorder-handle]');
    const row = handle?.closest<HTMLElement>('[data-reorder-id]');
    if (row) pointer = { id: event.pointerId, row, x: event.clientX, y: event.clientY };
  }
  function move(event: PointerEvent) {
    if (pointer?.id !== event.pointerId) return;
    if (options.disabled || !(event.buttons & 1) || !list.contains(pointer.row)) { reset(); return; }
    if (!drag) {
      if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) < 5 || rows().length < 2) return;
      drag = lift(pointer);
      list.setPointerCapture(event.pointerId);
      suppressClick = true;
      frame = requestAnimationFrame(scrollAtEdge);
    }
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    place(pointer.x, pointer.y);
  }
  async function up(event: PointerEvent) {
    if (pointer?.id !== event.pointerId) return;
    if (!drag || options.disabled) { reset(); return; }
    place(event.clientX, event.clientY);
    const { source, target, entries, preview, bounds, step, scrollTop } = drag;
    const id = entries[source].row.dataset.reorderId!;
    const destination = entries[target].row.dataset.reorderId!;
    const after = target > source;
    const top = entries[target].bounds.top - (after ? step - entries[target].bounds.height : 0)
      + scrollTop - scroll.scrollTop;
    pointer = undefined;
    cancelAnimationFrame(frame);
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches) preview.style.transition = 'transform 150ms ease-out';
    preview.style.transform = `translate(${bounds.left}px, ${top}px)`;
    try {
      if (source !== target && list.contains(entries[source].row) && list.contains(entries[target].row)) {
        await options.move(id, destination, after);
      }
    } finally { reset(); }
  }
  function click(event: MouseEvent) {
    if (suppressClick && event.detail) {
      suppressClick = false;
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }
  function key(event: KeyboardEvent) {
    if (event.key === 'Escape' && drag && pointer) {
      event.preventDefault();
      event.stopImmediatePropagation();
      reset();
      return;
    }
    if (drag || options.disabled || !event.altKey || !['ArrowUp', 'ArrowDown'].includes(event.key)) return;
    const row = (event.target as Element).closest('[data-reorder-id]');
    const entries = rows();
    const index = row instanceof HTMLElement ? entries.indexOf(row) : -1;
    if (index < 0) return;
    const below = event.key === 'ArrowDown';
    const next = entries[index + (below ? 1 : -1)];
    if (next) {
      event.preventDefault();
      void options.move(entries[index].dataset.reorderId!, next.dataset.reorderId!, below);
    }
  }
  const listeners = new AbortController();
  const signal = listeners.signal;
  list.addEventListener('pointerdown', down, { signal });
  window.addEventListener('pointermove', move, { signal });
  window.addEventListener('pointerup', up, { signal });
  window.addEventListener('pointercancel', reset, { signal });
  window.addEventListener('blur', reset, { signal });
  list.addEventListener('lostpointercapture', () => { if (pointer) reset(); }, { signal });
  list.addEventListener('click', click, { capture: true, signal });
  window.addEventListener('keydown', key, { capture: true, signal });
  list.addEventListener('dragstart', event => {
    if ((event.target as Element).closest('[data-reorder-handle]')) event.preventDefault();
  }, { signal });
  return {
    update(next) { options = next; },
    destroy() { reset(); listeners.abort(); },
  };
};
