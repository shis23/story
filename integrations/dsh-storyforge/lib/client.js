/**
 * dsh-plugin-storyforge — 浏览器半边（PoC）
 *
 * 隔离做法（DSH 0.2.0-rc.1）：
 * - 主区视图标签名单是全局的，没有按会话过滤的接口。会话头里挂一个"哨兵"按钮：
 *   当前会话是 StoryForge 会话（预设 = storyforge）时，用 layout effect 注册
 *   StoryForge 视图；离开时撤销。会话头同一时刻只有一份，所以最多一个哨兵在线。
 * - 同一个哨兵还用更低的 priority 盖住输入框的模型选择器（single 插槽），换成只读标签。
 * - 右栏页面类型不声明 guide：只出现在主动打开它的会话里（布局按会话持久化）。
 * - 会话头按钮、停靠区状态条在非 StoryForge 会话里渲染 null。
 * - 新建故事时，在打开会话**之前**写入视图偏好 `dsh.conversation.<id>`（整份状态），
 *   使新会话默认停在 StoryForge 视图。
 *
 * 产物是手写的 CJS 工厂包（宿主只吃已构建的 lib/client.js，不做编译）。
 */
window.__ModuleLoader__.load({
	id: "dsh-plugin-storyforge",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;
		Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });

		const React = require("react");
		const h = React.createElement;

		const API_BASE = "/api/storyforge";
		const PRESET = "storyforge";
		const VIEW_ID = "storyforge";
		const PANEL_KIND = "storyforge-panel";
		const PANEL_TYPE_ID = "dsh-plugin-storyforge.panel";
		const CONVERSATION_STORE_KEY = "dsh.conversation";
		const AUTO_PANEL_KEY = "storyforge.autoPanel.";
		const MODEL_LOCK_PRIORITY = -100;

		/** apply 时捕获的客户端根上下文（组件里用它访问服务）。 */
		let rootCtx = null;

		async function api(path, init) {
			const res = await fetch(`${API_BASE}${path}`, { credentials: "same-origin", ...init });
			const body = await res.json().catch(() => ({}));
			if (!res.ok || body.ok === false) throw new Error(body.error || `${path}: HTTP ${res.status}`);
			return body;
		}

		/** 当前会话是否是 StoryForge 会话（预设名是会话头里的不可变事实，分叉会继承）。 */
		function useIsStoryForge(props, sessionId) {
			const selector = (state) => state?.byId?.[sessionId]?.projectionValues?.agentPreset === PRESET;
			if (typeof props.useSessions === "function") return props.useSessions(selector);
			const list = rootCtx?.sessions?.list;
			return React.useSyncExternalStore(
				(onChange) => list?.subscribe?.(onChange) ?? (() => {}),
				() => (list ? selector(list.getSnapshot()) : false),
			);
		}

		//#region 动态注册（引用计数：第一个持有者注册，最后一个释放者撤销；撤销幂等）
		function dynamicRegistration(register) {
			let holders = 0;
			let dispose = null;
			const reset = () => {
				holders = 0;
				const current = dispose;
				dispose = null;
				if (typeof current === "function") current();
			};
			const acquire = () => {
				if (holders++ === 0) dispose = register();
				let released = false;
				return () => {
					if (released) return;
					released = true;
					if (--holders === 0) reset();
				};
			};
			return { acquire, reset };
		}

		/** 主区 StoryForge 视图标签。 */
		const storyView = dynamicRegistration(() =>
			rootCtx.slots.register(
				{
					name: "conversation.view",
					id: VIEW_ID,
					order: 5,
					label: "StoryForge",
					inject: (sessionId) => ({ sfSessionId: sessionId }),
				},
				StoryForgeView,
			),
		);

		/**
		 * 输入框的模型位：single 插槽，按 priority 升序最低者渲染，原生选择器在 0。
		 * StoryForge 会话里用只读标签盖住它——在这里改模型只会被钉回管线，却会让 DSH
		 * 每轮追加一条"换模型"通知，并顺带改掉全局默认模型。
		 */
		const modelLock = dynamicRegistration(() =>
			rootCtx.slots.register(
				{
					name: "conversation.input.model",
					priority: MODEL_LOCK_PRIORITY,
					inject: (sessionId) => ({ sfSessionId: sessionId }),
				},
				ModelLock,
			),
		);
		//#endregion

		const muted = { opacity: 0.6, fontSize: 12 };
		const buttonStyle = {
			all: "unset",
			cursor: "pointer",
			padding: "3px 10px",
			borderRadius: 6,
			fontSize: 12,
			border: "1px solid color-mix(in srgb, currentColor 25%, transparent)",
		};
		const cellStyle = {
			textAlign: "left",
			padding: "4px 8px",
			borderBottom: "1px solid color-mix(in srgb, currentColor 12%, transparent)",
			verticalAlign: "top",
		};

		function openPanel() {
			try {
				rootCtx.sidebarRight.openTab(PANEL_KIND);
			} catch (e) {
				console.error("[storyforge] open panel failed", e);
			}
		}

		/** 会话头按钮兼哨兵：StoryForge 会话里注册视图与模型锁并提供"面板"按钮，其它会话渲染 null。 */
		function HeaderAction(props) {
			const sessionId = props.sessionId ?? props.sfSessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);

			React.useLayoutEffect(() => {
				if (!isStoryForge) return undefined;
				const releaseView = storyView.acquire();
				const releaseLock = modelLock.acquire();
				return () => {
					releaseLock();
					releaseView();
				};
			}, [isStoryForge]);

			// 新建故事后首次进入：等右栏挂到本会话再打开面板，只做一次。
			React.useEffect(() => {
				if (!isStoryForge || localStorage.getItem(AUTO_PANEL_KEY + sessionId) !== "1") return;
				const mounted = rootCtx.sidebarRight?.mounted;
				const tryOpen = () => {
					if (mounted?.getSnapshot?.() !== sessionId) return false;
					localStorage.removeItem(AUTO_PANEL_KEY + sessionId);
					openPanel();
					return true;
				};
				if (tryOpen()) return;
				const unsubscribe = mounted?.subscribe?.(() => {
					if (tryOpen()) unsubscribe?.();
				});
				return () => unsubscribe?.();
			}, [isStoryForge, sessionId]);

			if (!isStoryForge) return null;
			return h(
				"button",
				{ type: "button", style: buttonStyle, title: "打开 StoryForge 面板", onClick: openPanel },
				"StoryForge 面板",
			);
		}

		/** 主区 StoryForge 视图（P1 只做占位，验证可见性与切换）。 */
		function StoryForgeView(props) {
			const sessionId = props.sfSessionId ?? props.sessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);
			return h("div", { style: { padding: "20px 24px", fontSize: 14, lineHeight: 1.7 } }, [
				h("div", { key: "t", style: { fontSize: 16, fontWeight: 600, marginBottom: 8 } }, "StoryForge 写作视图（PoC 占位）"),
				!isStoryForge &&
					h("div", { key: "warn", style: { color: "#e57373", marginBottom: 8 } }, "此视图仅用于 StoryForge 会话。"),
				h("div", { key: "id", style: muted }, `会话：${sessionId}`),
				h(
					"p",
					{ key: "p" },
					"P2 起这里会是 StoryForge 的写作界面（正文、角色状态、选项）。当前正文请看「对话」标签。",
				),
				h("button", { key: "b", type: "button", style: buttonStyle, onClick: openPanel }, "打开 StoryForge 面板"),
			]);
		}

		/** 右栏 StoryForge 面板。 */
		function PanelTab(props) {
			const sessionId = props.sessionId ?? rootCtx.sidebarRight?.mounted?.getSnapshot?.();
			const isStoryForge = useIsStoryForge(props, sessionId);
			const [info, setInfo] = React.useState(null);
			React.useEffect(() => {
				if (!isStoryForge) return;
				api("/sessions")
					.then((body) => setInfo(body.sessions?.[sessionId] ?? null))
					.catch(() => setInfo(null));
			}, [isStoryForge, sessionId]);
			if (!isStoryForge) {
				return h("div", { style: { padding: 16, ...muted } }, "此面板仅用于 StoryForge 会话。");
			}
			return h("div", { style: { padding: 16, fontSize: 13, lineHeight: 1.7 } }, [
				h("div", { key: "t", style: { fontWeight: 600, marginBottom: 6 } }, "StoryForge"),
				h("div", { key: "s", style: muted }, `会话：${sessionId}`),
				h(
					"div",
					{ key: "i" },
					info
						? `故事：${info.title}${info.forkedFrom ? `（分叉自 ${info.forkedFrom}）` : ""}`
						: "未登记的 StoryForge 会话（例如从新会话页直接选了预设）",
				),
				h("div", { key: "n", style: { ...muted, marginTop: 8 } }, "P2 起这里放角色、知识、变量、后处理日志。"),
			]);
		}

		/** 输入框模型位的只读标签（只在 StoryForge 会话里注册）。 */
		function ModelLock() {
			return h(
				"span",
				{
					style: { ...muted, display: "var(--dsh-composer-model-text-display, inline)", whiteSpace: "nowrap", padding: "0 6px" },
					title: "StoryForge 会话的模型固定为 StoryForge 管线",
				},
				"StoryForge 管线",
			);
		}

		/** 停靠区状态条（输入框下方）。 */
		function DockStatus(props) {
			const sessionId = props.sfSessionId ?? props.sessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);
			if (!isStoryForge) return null;
			return h("div", { style: { ...muted, padding: "2px 4px" } }, "StoryForge · 假管线 · 后处理：空闲");
		}

		/** 主区首页：新建故事 + 调试信息。 */
		function HomePanel() {
			const [calls, setCalls] = React.useState([]);
			const [events, setEvents] = React.useState([]);
			const [busy, setBusy] = React.useState(false);
			const [error, setError] = React.useState(null);

			const refresh = React.useCallback(async () => {
				try {
					const [c, l] = await Promise.all([api("/debug/calls"), api("/debug/lifecycle")]);
					setCalls(c.calls ?? []);
					setEvents(l.events ?? []);
					setError(null);
				} catch (e) {
					setError(String(e?.message ?? e));
				}
			}, []);

			React.useEffect(() => {
				void refresh();
			}, [refresh]);

			const createStory = async () => {
				setBusy(true);
				try {
					const { sessionId } = await api("/sessions", {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: JSON.stringify({ title: `故事 ${new Date().toLocaleTimeString()}` }),
					});
					// 打开会话前写入整份会话状态：新会话默认停在 StoryForge 视图。
					localStorage.setItem(
						`${CONVERSATION_STORE_KEY}.${sessionId}`,
						JSON.stringify({ draft: "", view: VIEW_ID, viewRequest: null }),
					);
					localStorage.setItem(AUTO_PANEL_KEY + sessionId, "1");
					rootCtx.uiWorkspace.openSession(sessionId);
				} catch (e) {
					setError(String(e?.message ?? e));
				} finally {
					setBusy(false);
				}
			};

			const header = ["时间", "会话", "用途", "模型", "消息数", "工具", "开场", "跳过的注入"];
			return h("div", { style: { height: "100%", boxSizing: "border-box", overflow: "auto", padding: "16px 20px", fontSize: 13 } }, [
				h("div", { key: "head", style: { display: "flex", alignItems: "center", gap: 12, marginBottom: 12 } }, [
					h("span", { key: "t", style: { fontSize: 15, fontWeight: 600 } }, "StoryForge"),
					h("button", { key: "n", type: "button", style: buttonStyle, disabled: busy, onClick: () => void createStory() }, busy ? "创建中…" : "新建故事"),
					h("button", { key: "r", type: "button", style: buttonStyle, onClick: () => void refresh() }, "刷新调试信息"),
				]),
				error && h("div", { key: "err", style: { color: "#e57373", marginBottom: 8 } }, error),
				h("div", { key: "ct", style: { fontWeight: 600, margin: "8px 0 4px" } }, "最近的适配器调用"),
				h("table", { key: "calls", style: { width: "100%", borderCollapse: "collapse", fontVariantNumeric: "tabular-nums" } }, [
					h("thead", { key: "h" }, h("tr", null, header.map((label) => h("th", { key: label, style: cellStyle }, label)))),
					h(
						"tbody",
						{ key: "b" },
						calls.length === 0
							? h("tr", null, h("td", { colSpan: header.length, style: cellStyle }, "还没有调用"))
							: calls.map((call, i) =>
									h("tr", { key: i }, [
										h("td", { key: "at", style: cellStyle }, call.at.slice(11, 19)),
										h("td", { key: "s", style: cellStyle }, call.sessionId ?? "—"),
										h("td", { key: "p", style: cellStyle }, call.purpose ?? "对话"),
										h("td", { key: "m", style: cellStyle }, `${call.provider}/${call.model}`),
										h("td", { key: "n", style: cellStyle }, String(call.messageCount)),
										h("td", { key: "tools", style: cellStyle }, call.toolNames.length === 0 ? "无" : `${call.toolNames.length} 个`),
										h("td", { key: "o", style: cellStyle }, call.opening ? "是" : ""),
										h("td", { key: "notice", style: cellStyle }, (call.injected ?? []).join("、")),
									]),
								),
					),
				]),
				h("div", { key: "lt", style: { fontWeight: 600, margin: "16px 0 4px" } }, "StoryForge 会话生命周期"),
				h(
					"ul",
					{ key: "events", style: { margin: 0, paddingLeft: 18 } },
					events.length === 0
						? h("li", { style: muted }, "暂无")
						: events.map((event, i) =>
								h(
									"li",
									{ key: i },
									`${event.at.slice(11, 19)} ${event.sessionId} source=${event.source}${event.parentSession ? ` 分叉自 ${event.parentSession}` : ""}`,
								),
							),
				),
			]);
		}

		/** 左侧栏图标：一本打开的书。 */
		function PanelIcon(props) {
			const size = props?.size ?? 18;
			return h(
				"svg",
				{ width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.6, "aria-hidden": "true" },
				[
					h("path", { key: "l", d: "M12 6c-2-1.5-5-2-8-1.5v13c3-.5 6 0 8 1.5", strokeLinejoin: "round" }),
					h("path", { key: "r", d: "M12 6c2-1.5 5-2 8-1.5v13c-3-.5-6 0-8 1.5V6z", strokeLinejoin: "round" }),
				],
			);
		}

		const inject = ["slots", "sessions", "sidebarRight", "sidebarRightTabs", "uiWorkspace"];

		function apply(ctx) {
			rootCtx = ctx;

			ctx.slots.inject("sidebar.panellist", () =>
				ctx.slots.register({ name: "sidebar.panellist", id: "storyforge", order: 13, label: "StoryForge" }, PanelIcon),
			);
			ctx.slots.inject("main", () => ctx.slots.register({ name: "main", key: "storyforge" }, HomePanel));

			ctx.slots.inject("conversation.session.header.actions", () =>
				ctx.slots.register(
					{
						name: "conversation.session.header.actions",
						id: "storyforge-sentinel",
						order: 900,
						inject: (sessionId) => ({ sfSessionId: sessionId }),
					},
					HeaderAction,
				),
			);

			ctx.slots.inject("conversation.composer.dock", () =>
				ctx.slots.register(
					{
						name: "conversation.composer.dock",
						id: "storyforge-status",
						order: 900,
						inject: (sessionId) => ({ sfSessionId: sessionId }),
					},
					DockStatus,
				),
			);

			// 右栏页面类型：不声明 guide，只在主动打开它的会话里出现。
			ctx.effect(
				() => ctx.sidebarRightTabs.register({ id: PANEL_TYPE_ID, kind: PANEL_KIND, title: () => "StoryForge" }),
				"storyforge: sidebar panel type",
			);
			ctx.slots.inject("sidebar.right.pane.tab", () =>
				ctx.slots.register({ name: "sidebar.right.pane.tab", key: PANEL_TYPE_ID }, PanelTab),
			);

			// 插件卸载时撤销仍在注册中的视图与模型锁。
			ctx.effect(
				() => () => {
					modelLock.reset();
					storyView.reset();
				},
				"storyforge: dynamic registration cleanup",
			);
		}

		exports.apply = apply;
		exports.inject = inject;
		return module.exports;
	},
});
