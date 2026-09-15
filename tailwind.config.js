/**
 * 设计令牌唯一来源。
 *
 * 【为什么要有这个文件】此前 `bg-panel-soft`、`text-ink-faint` 这类类名
 * 已经在组件里用了 6 处，但令牌从未在这里定义——**Tailwind 对未知令牌是静默忽略的**，
 * 不报错、不警告，只是那条样式不生效。后果很隐蔽：
 * `text-ink-faint` 失效后文字回退到继承色（最亮的 ink），
 * 本该"最淡的补充说明"反而变成整屏最扎眼的东西，视觉层级整个反过来。
 * 所以令牌必须集中定义，并由 `npm run ui:check` 静态守住。
 *
 * @type {import('tailwindcss').Config}
 */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // ——— 深色底色层级：面板 → 面板内嵌 → 卡片 → 悬浮 ———
        base: '#0d1117',
        panel: '#161b22',
        /** 嵌在面板内部的下沉块（比 panel 略亮，比 elevated 略暗） */
        'panel-soft': '#1a2029',
        elevated: '#1c2128',
        hover: '#21262d',

        // ——— 描边 ———
        line: '#30363d',
        'line-subtle': '#21262d',
        /** 需要强调分隔（如拖拽落点）时用 */
        'line-strong': '#484f58',

        // ——— 文字层级：由亮到暗，共四级 ———
        ink: '#e6edf3',
        'ink-muted': '#8b949e',
        'ink-dim': '#6e7681',
        /** 第四级：脚注、时间戳这类"看得见但不该抢注意力"的文字 */
        'ink-faint': '#565d66',

        // ——— 状态色 ———
        ok: '#3fb950',
        warn: '#d29922',
        danger: '#f85149',
        accent: '#58a6ff',
        purple: '#bc8cff',
        /**
         * 编排专用色。
         *
         * 刻意与四档风险色区分开：风险色回答"这东西有没有问题"，
         * 编排色回答"这是你计划要改动的东西"。两件事混用同一种颜色，
         * 用户就分不清屏幕上哪些是**现状**、哪些是**自己的意图**。
         */
        plan: '#a371f7',
      },
      fontFamily: {
        sans: ['Inter', 'system-ui', 'Segoe UI', 'Microsoft YaHei', 'sans-serif'],
        mono: ['Cascadia Code', 'Consolas', 'JetBrains Mono', 'monospace'],
      },
      fontSize: {
        /*
         * 只保留三级小字号，取代散落各处的 text-[10px] / text-[11px] / leading-[14px]。
         * 刻意不新增与 '2xs' 尺寸重复的别名——同一尺寸有两条名字，
         * 下一个改动的人就不知道该用哪条，令牌体系会重新烂掉。
         *
         * 全部用 rem + 无单位行高：clamp 后的根字号（见 index.css :root）
         * 会把这一整套文字随屏宽平滑缩放，1080p 上收敛、2K/4K 上放大，
         * 各分辨率下的层级关系保持一致。根字号为 16px 时与旧 px 值完全等价。
         */
        '2xs': ['0.625rem', '1.4'],
        mini: ['0.6875rem', '1.4545'],
        body: ['0.75rem', '1.5'],
      },
      borderRadius: {
        card: '6px',
      },
      boxShadow: {
        /** 浮层（变更篮展开、拖拽卡）用 */
        float: '0 8px 24px rgba(1, 4, 9, 0.6), 0 0 0 1px #30363d',
        /** 键盘焦点环，全局统一 */
        focus: '0 0 0 2px rgba(88, 166, 255, 0.4)',
      },
      transitionDuration: {
        fast: '120ms',
        base: '180ms',
      },
      keyframes: {
        'slide-up': {
          from: { transform: 'translateY(6px)', opacity: '0' },
          to: { transform: 'translateY(0)', opacity: '1' },
        },
        'flash-change': {
          '0%': { backgroundColor: 'rgba(163, 113, 247, 0.18)' },
          '100%': { backgroundColor: 'transparent' },
        },
      },
      animation: {
        'slide-up': 'slide-up 180ms cubic-bezier(0.2, 0, 0, 1)',
        'flash-change': 'flash-change 900ms ease-out',
      },
    },
  },
  plugins: [],
}
