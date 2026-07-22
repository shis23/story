# Campaign 世界书 + 完整 Card Shell WebView

> 状态：执行中（2026-07-22）
>
> **进度**：Phase 0–2 已落地（活动世界书存储/UI、卡只读、Meta 写活动书）。Phase 3–4 已落地提取器 + allowlist 缓存 + 可见 `CardShellHost`（写作屏 #shell 挂 status/opening）。Phase 5–7（display 分流、TH 顺序执行、证据截图）进行中。  
> 硬约束：**不接受降级**（禁止用纯文本开场 / 禁网隐藏 iframe / 仅声明式状态栏冒充完成）。  
> 金标卡：仓库根 `test-card.png`（命定之诗与黄昏之歌 v4.1）。

## 分层

```text
角色卡世界书 = 模板只读
Campaign 世界书 = 本局真相源（可读写）
写作注入 = 活动书
CardShellHost = 可见 WebView + 宿主代持远程资源（开场/状态/消息壳）
MvuJsRuntime = 隐藏片段执行（并存，不替代壳）
```

## test-card 壳清单（提取器黄金断言）

| 用途 | URL / 形态 |
| --- | --- |
| 首页 | `…/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html` |
| 自定义开局 | `…/dist/custom_start/index.html` |
| 状态栏 | `…/dist/status/index.html` |
| MVU | MagVarUpdate `bundle.js` + data_schema |
| 其它 TH | 自动化 / 预载 / 创意工坊 inline / AutoDialogueBeautifier |

Display 正则将 `【首页】` / `<customized>` / `<StatusPlaceHolderImpl/>` 替换为 `$('body').load(url)` 脚本；完整渲染必须执行该语义（宿主代持 fetch + 注入），禁止 RichContent 禁 script 当完成。

## 阶段

0. 本文件 + 壳清单固化  
1. Campaign 世界书存储 / 开档拷贝 / 注入 / API  
2. 卡只读 UI + 活动世界书 Tab  
3. 壳资源提取 + allowlist 缓存后端  
4. CardShellHost 可见面 + jQuery.load 宿主实现  
5. 消息 display 分流挂载 + 变量出站  
6. tavern_helper 远程 module  
7. Meta patch 写活动书 + 回归证据  

## 非目标

- ST 99 事件全集  
- knowledge 冒充世界书  
- 宣称插件生态 100% 等价（但 test-card 主路径三壳必须可玩）
