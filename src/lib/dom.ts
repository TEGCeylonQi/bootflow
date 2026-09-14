/**
 * 少量直接读 DOM 的工具函数。
 *
 * 【为什么这里破例去碰 DOM，而不是从 store 里算】
 * "当前可见项的先后顺序"由 `useFilteredItems` → 分组 → 组内排序三层共同决定。
 * 如果键盘导航、范围选择各自再算一遍，就会有三份可能不一致的实现，
 * 而它们的落点必须与用户眼睛看到的完全一致。
 *
 * 直接按渲染结果的顺序读，天然不会有偏差，也省掉了三处重复的排序逻辑。
 */

/** 当前可见（且已渲染）的项 id，顺序即屏幕上从上到下的顺序 */
export function visibleItemIds(): string[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-item-row]'))
    .map((el) => el.dataset.itemRow ?? '')
    .filter(Boolean)
}
