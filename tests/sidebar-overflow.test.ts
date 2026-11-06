import assert from 'node:assert/strict';
import test from 'node:test';
import { sidebarVisibleCount } from '../official-plugins/ui.shell/frontend/lib/sidebar-overflow.ts';

test('sidebar overflow preserves a More row as contributions or theme sizes grow', () => {
  assert.equal(sidebarVisibleCount(0, 200, 48, 8, 3), 0);
  assert.equal(sidebarVisibleCount(3, 160, 48, 8, 3), 3, 'exact fit needs no More row');
  assert.equal(sidebarVisibleCount(4, 160, 48, 8, 3), 2, 'More consumes the third available row');
  assert.equal(sidebarVisibleCount(20, 400, 48, 8, 3), 3, 'large lists retain a compact sidebar');
  assert.equal(sidebarVisibleCount(3, 160, 72, 8, 3), 1, 'larger themed buttons move into More');
  assert.equal(sidebarVisibleCount(3, 48, 48, 8, 3), 0, 'small windows keep More as the only plugin row');
  assert.equal(sidebarVisibleCount(3, -20, 48, 8, 3), 0);
  assert.equal(sidebarVisibleCount(2, 200, 48, 8, 1), 1, 'footer contributions keep their own compact limit');
});
