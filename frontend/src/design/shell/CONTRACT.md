# 应用壳 · 接线契约卡（design/shell）

`AppFrame` 纯布局。侧栏导航项 / 顶栏状态徽章由 adapter 从 store 组装后传给
展示子组件（或继续复用已换肤的 `PrimarySidebar` / `TopBar` 直至其迁入 design/）。

当前生产仍用 `components-v2/shell/AppShell.vue`（已纸面 token 化）；
`AppFrame` 供预览与后续整壳切换，避免一次替换打断插件 host 挂载点。
