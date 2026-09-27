/** 禁用浏览器默认右键菜单。
 * 以后要做自定义右键菜单的元素,加 `data-allow-context` 属性即可放行(在此挂自定义菜单)。 */
export function disableDefaultContextMenu(): void {
  document.addEventListener('contextmenu', (e) => {
    const el = e.target as HTMLElement | null;
    if (el?.closest?.('[data-allow-context]')) return;
    e.preventDefault();
  });
  // 输入框保留原生菜单(复制/粘贴还得能用)
  document.addEventListener(
    'contextmenu',
    (e) => {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) {
        e.stopPropagation();
      }
    },
    true,
  );
}
