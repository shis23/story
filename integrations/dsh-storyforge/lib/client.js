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
 *
 * P2：首页选底层模型（StoryForge 管线内部每次模型调用都用它）与角色卡，新建故事
 * 时由 sidecar 导卡、识别角色、开档；停靠条与面板显示 sidecar 收尾状态。
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

		/** 定时拉取 `/status`（sidecar 状态、收尾报告）；组件卸载即停。 */
		function useBridgeStatus(enabled, intervalMs = 3000) {
			const [status, setStatus] = React.useState(null);
			React.useEffect(() => {
				if (!enabled) return undefined;
				let alive = true;
				const load = () =>
					api("/status")
						.then((body) => alive && setStatus(body))
						.catch(() => alive && setStatus(null));
				void load();
				const timer = setInterval(load, intervalMs);
				return () => {
					alive = false;
					clearInterval(timer);
				};
			}, [enabled, intervalMs]);
			return status;
		}

		/** 本会话登记的故事信息（`/sessions` 里的一项）。 */
		function useStoryInfo(enabled, sessionId) {
			const [info, setInfo] = React.useState(null);
			React.useEffect(() => {
				if (!enabled) return;
				api("/sessions")
					.then((body) => setInfo(body.sessions?.[sessionId] ?? null))
					.catch(() => setInfo(null));
			}, [enabled, sessionId]);
			return info;
		}

		const ACCEPT_LABELS = { accepted: "已接受", accepted_degraded: "已接受（质量降级）", failed: "接受失败" };
		const POSTPROCESS_LABELS = {
			PostProcessDone: "后处理完成",
			PostProcessFailed: "后处理失败",
			PostProcessSkipped: "后处理跳过",
		};

		function describeSettle(report) {
			if (!report) return "—";
			const parts = [POSTPROCESS_LABELS[report.postprocess] ?? "后处理无终态", ACCEPT_LABELS[report.accept] ?? report.accept];
			if (report.accept_error) parts.push(report.accept_error);
			if (report.usage?.calls) parts.push(`后处理 ${describeUsage(report.usage)}`);
			return parts.join(" · ");
		}

		/** sidecar 的 UsageTotals → 一行文字。 */
		function describeUsage(usage) {
			if (!usage) return "—";
			const failed = usage.failed_calls ? `（失败 ${usage.failed_calls}）` : "";
			const cached = usage.cache_read_tokens ? ` 缓存 ${usage.cache_read_tokens}` : "";
			return `${usage.calls} 次${failed} · 入 ${usage.input_tokens} 出 ${usage.output_tokens}${cached}`;
		}

		const LLM_STATUS_LABELS = { running: "进行中", ok: "完成", error: "失败", aborted: "已中止" };

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

		/** 主区 StoryForge 视图（占位：验证可见性与切换；写作界面留到后续阶段）。 */
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
					"StoryForge 自己的写作界面（正文、角色状态、选项）留到后续阶段。正文与管线进度请看「对话」标签：思考块里是导演 / 角色 / 执笔的过程，回复正文是质量门之后的成文。",
				),
				h("button", { key: "b", type: "button", style: buttonStyle, onClick: openPanel }, "打开 StoryForge 面板"),
			]);
		}

		/** 右栏 StoryForge 面板。 */
		function PanelTab(props) {
			const sessionId = props.sessionId ?? rootCtx.sidebarRight?.mounted?.getSnapshot?.();
			const isStoryForge = useIsStoryForge(props, sessionId);
			const info = useStoryInfo(isStoryForge, sessionId);
			const status = useBridgeStatus(isStoryForge);
			if (!isStoryForge) {
				return h("div", { style: { padding: 16, ...muted } }, "此面板仅用于 StoryForge 会话。");
			}
			const settles = (status?.settles ?? []).filter((report) => info?.campaignId && report.campaign_id === info.campaignId);
			const row = (key, label, value) =>
				h("div", { key }, [h("span", { key: "l", style: muted }, `${label}：`), h("span", { key: "v" }, value ?? "—")]);
			return h("div", { style: { padding: 16, fontSize: 13, lineHeight: 1.7 } }, [
				h("div", { key: "t", style: { fontWeight: 600, marginBottom: 6 } }, info?.title ?? "StoryForge"),
				info
					? h("div", { key: "info" }, [
							row("card", "角色卡", info.cardName ? `${info.cardName}（${info.cardFile ?? ""}）` : null),
							row("campaign", "存档", info.campaignId),
							row("extract", "角色识别", info.extractionStatus),
							info.forkedFrom && row("fork", "分叉自", info.forkedFrom),
						])
					: h("div", { key: "none", style: muted }, "未登记的 StoryForge 会话（例如从新会话页直接选了预设）"),
				h("div", { key: "ht", style: { fontWeight: 600, margin: "12px 0 4px" } }, "最近几轮的收尾"),
				settles.length === 0
					? h("div", { key: "empty", style: muted }, "暂无")
					: h(
							"ul",
							{ key: "settles", style: { margin: 0, paddingLeft: 18 } },
							settles.map((report, i) => h("li", { key: i }, `${report.at.slice(11, 19)} ${describeSettle(report)}`)),
						),
				h("div", { key: "n", style: { ...muted, marginTop: 12 } }, "角色、知识、变量的查看与编辑留到后续阶段。"),
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
			const info = useStoryInfo(isStoryForge, sessionId);
			const status = useBridgeStatus(isStoryForge);
			if (!isStoryForge) return null;
			if (status?.pipeline === "fake") return h("div", { style: { ...muted, padding: "2px 4px" } }, "StoryForge · 假管线");
			const model = status?.model ? `${status.model.provider}/${status.model.model}` : "未选底层模型";
			const busy = status?.status?.busy ? "写作 / 收尾中" : "空闲";
			const last = (status?.settles ?? []).find((report) => info?.campaignId && report.campaign_id === info.campaignId);
			const text = `StoryForge · ${model} · ${status?.bridge?.running ? busy : "sidecar 未启动"}${last ? ` · 上一轮：${describeSettle(last)}` : ""}`;
			return h("div", { style: { ...muted, padding: "2px 4px" } }, text);
		}

		/** 主区首页：底层模型、新建故事、sidecar 状态与调试信息。 */
		function HomePanel() {
			const [calls, setCalls] = React.useState([]);
			const [events, setEvents] = React.useState([]);
			const [models, setModels] = React.useState(null);
			const [cards, setCards] = React.useState(null);
			const [card, setCard] = React.useState("");
			const [title, setTitle] = React.useState("");
			const [busy, setBusy] = React.useState(false);
			const [error, setError] = React.useState(null);
			const status = useBridgeStatus(true, 4000);
			const fake = status?.pipeline === "fake";

			const refresh = React.useCallback(async () => {
				try {
					const [c, l, m, k] = await Promise.all([api("/debug/calls"), api("/debug/lifecycle"), api("/models"), api("/cards")]);
					setCalls(c.calls ?? []);
					setEvents(l.events ?? []);
					setModels(m);
					setCards(k);
					setCard((current) => current || k.cards?.[0]?.file || "");
					setError(null);
				} catch (e) {
					setError(String(e?.message ?? e));
				}
			}, []);

			React.useEffect(() => {
				void refresh();
			}, [refresh]);

			/** 下拉框的选项与 { provider, model } 一一对应（value 用序号，避免拼接 id）。 */
			const choices = React.useMemo(
				() =>
					(models?.providers ?? []).flatMap((provider) =>
						provider.models.map((model) => ({ provider: provider.provider, model: model.id })),
					),
				[models],
			);

			const saveSetting = async (patch) => {
				try {
					await api("/settings", {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: JSON.stringify(patch),
					});
					setModels((current) => ({ ...current, ...patch }));
				} catch (e) {
					setError(String(e?.message ?? e));
				}
			};
			const optionSelect = (key, labels, value, onChange) =>
				h(
					"select",
					{ key, value: value ?? "", onChange: (e) => void onChange(e.target.value) },
					Object.entries(labels ?? {}).map(([id, label]) => h("option", { key: id, value: id }, label)),
				);

			const selectModel = async (index) => {
				const choice = choices[Number(index)];
				if (!choice) return;
				try {
					await api("/settings", {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: JSON.stringify(choice),
					});
					setModels((current) => ({ ...current, selected: choice }));
				} catch (e) {
					setError(String(e?.message ?? e));
				}
			};

			const createStory = async () => {
				setBusy(true);
				setError(null);
				try {
					const { sessionId } = await api("/sessions", {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: JSON.stringify({ title: title.trim() || undefined, card }),
					});
					// 打开会话前写入整份会话状态：新会话默认停在 StoryForge 视图。
					localStorage.setItem(
						`${CONVERSATION_STORE_KEY}.${sessionId}`,
						JSON.stringify({ draft: "", view: VIEW_ID, viewRequest: null }),
					);
					localStorage.setItem(AUTO_PANEL_KEY + sessionId, "1");
					setTitle("");
					rootCtx.uiWorkspace.openSession(sessionId);
				} catch (e) {
					setError(String(e?.message ?? e));
				} finally {
					setBusy(false);
				}
			};

			const selected = models?.selected;
			const selectedIndex = selected
				? choices.findIndex((choice) => choice.provider === selected.provider && choice.model === selected.model)
				: -1;
			let optionIndex = 0;
			const modelSelect = h(
				"select",
				{
					key: "model",
					value: selectedIndex >= 0 ? String(selectedIndex) : "",
					onChange: (e) => void selectModel(e.target.value),
					style: { maxWidth: 360 },
				},
				[
					h(
						"option",
						{ key: "none", value: "", disabled: true },
						models ? (selected && selectedIndex < 0 ? `${selected.provider}/${selected.model}（已不可用）` : "选择底层模型…") : "加载中…",
					),
					...(models?.providers ?? []).map((provider) =>
						h(
							"optgroup",
							{ key: provider.provider, label: provider.error ? `${provider.name}（${provider.error}）` : provider.name },
							provider.models.map((model) => h("option", { key: model.id, value: String(optionIndex++) }, model.name)),
						),
					),
				],
			);
			const cardSelect = h(
				"select",
				{ key: "card", value: card, onChange: (e) => setCard(e.target.value), style: { maxWidth: 260 } },
				(cards?.cards ?? []).length === 0
					? [h("option", { key: "none", value: "" }, "（卡目录为空）")]
					: cards.cards.map((entry) => h("option", { key: entry.file, value: entry.file }, entry.file)),
			);
			const canCreate = !busy && (fake || Boolean(card && selected));

			const header = ["时间", "会话", "用途", "模型", "消息数", "工具", "开场", "跳过的注入"];
			const settleHeader = ["时间", "存档", "后处理", "接受", "后处理用量"];
			const llmHeader = ["#", "时间", "模型", "消息", "工具", "状态", "耗时", "输入 / 输出", "工具调用"];
			const llmCalls = status?.llmCalls ?? [];
			const section = (key, text) => h("div", { key, style: { fontWeight: 600, margin: "16px 0 4px" } }, text);
			const bridgeLine = !status
				? "加载中…"
				: fake
					? "假管线模式（不启动 sidecar）"
					: status.bridge.running
						? `运行中 pid=${status.bridge.pid} · 活跃存档 ${status.status?.active_campaign ?? "无"} · ${status.status?.busy ? "忙" : "空闲"}${status.statusError ? ` · ${status.statusError}` : ""}`
						: `未启动（第一次新建故事时拉起）${status.bridge.lastExit ? ` · 上次退出 code=${status.bridge.lastExit.code}` : ""} · 日志 ${status.bridge.logFile}`;
			return h("div", { style: { height: "100%", boxSizing: "border-box", overflow: "auto", padding: "16px 20px", fontSize: 13 } }, [
				h("div", { key: "head", style: { display: "flex", alignItems: "center", gap: 12, marginBottom: 12 } }, [
					h("span", { key: "t", style: { fontSize: 15, fontWeight: 600 } }, "StoryForge"),
					h("button", { key: "r", type: "button", style: buttonStyle, onClick: () => void refresh() }, "刷新"),
				]),
				error && h("div", { key: "err", style: { color: "#e57373", marginBottom: 8, whiteSpace: "pre-wrap" } }, error),
				!fake &&
					h("div", { key: "m", style: { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" } }, [
						h("span", { key: "l" }, "底层模型"),
						modelSelect,
						h("span", { key: "hint", style: muted }, "导演、角色、编剧、后处理共用，只在 StoryForge 管线内部使用"),
					]),
				!fake &&
					models &&
					h("div", { key: "modes", style: { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap", marginTop: 10 } }, [
						h("span", { key: "gl" }, "生成模式"),
						optionSelect("gm", models.generationModes, models.generationMode, (value) => saveSetting({ generationMode: value })),
						h("span", { key: "tl" }, "工具协议"),
						optionSelect("tm", models.toolModes, models.toolMode, (value) => saveSetting({ toolMode: value })),
						h(
							"span",
							{ key: "hint", style: muted },
							"自动路由遇到高成本群像会要求显式选择；文本降级给不支持工具调用的模型用",
						),
					]),
				h("div", { key: "new", style: { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap", marginTop: 10 } }, [
					!fake && h("span", { key: "l" }, "角色卡"),
					!fake && cardSelect,
					h("input", {
						key: "title",
						value: title,
						placeholder: "故事标题（可空，默认用卡名）",
						onChange: (e) => setTitle(e.target.value),
						style: { width: 220 },
					}),
					h(
						"button",
						{ key: "n", type: "button", style: buttonStyle, disabled: !canCreate, onClick: () => void createStory() },
						busy ? (fake ? "创建中…" : "导卡、识别角色中…") : "新建故事",
					),
				]),
				!fake && cards && h("div", { key: "cd", style: { ...muted, marginTop: 4 } }, `把 .png / .json 角色卡放进 ${cards.cardsDir}`),
				section("st", "sidecar"),
				h("div", { key: "status", style: muted }, bridgeLine),
				section("sht", "最近的收尾（后处理 + 自动接受）"),
				h("table", { key: "settles", style: { width: "100%", borderCollapse: "collapse" } }, [
					h("thead", { key: "h" }, h("tr", null, settleHeader.map((label) => h("th", { key: label, style: cellStyle }, label)))),
					h(
						"tbody",
						{ key: "b" },
						(status?.settles ?? []).length === 0
							? h("tr", null, h("td", { colSpan: settleHeader.length, style: cellStyle }, "暂无"))
							: status.settles.map((report, i) =>
									h("tr", { key: i }, [
										h("td", { key: "at", style: cellStyle }, report.at.slice(11, 19)),
										h("td", { key: "c", style: cellStyle }, report.campaign_id),
										h(
											"td",
											{ key: "p", style: cellStyle },
											`${POSTPROCESS_LABELS[report.postprocess] ?? "—"}${report.postprocess_detail ? `：${report.postprocess_detail}` : ""}`,
										),
										h(
											"td",
											{ key: "a", style: cellStyle },
											`${ACCEPT_LABELS[report.accept] ?? report.accept}${report.accept_error ? `：${report.accept_error}` : ""}`,
										),
										h("td", { key: "u", style: cellStyle }, describeUsage(report.usage)),
									]),
								),
					),
				]),
				section("llm", `底层模型调用（sidecar 经插件发出，共 ${status?.llmCallCount ?? 0} 次，显示最近 ${llmCalls.length} 次）`),
				h("table", { key: "llmTable", style: { width: "100%", borderCollapse: "collapse", fontVariantNumeric: "tabular-nums" } }, [
					h("thead", { key: "h" }, h("tr", null, llmHeader.map((label) => h("th", { key: label, style: cellStyle }, label)))),
					h(
						"tbody",
						{ key: "b" },
						llmCalls.length === 0
							? h("tr", null, h("td", { colSpan: llmHeader.length, style: cellStyle }, "还没有调用"))
							: llmCalls.map((call) =>
									h("tr", { key: call.seq }, [
										h("td", { key: "n", style: cellStyle }, String(call.seq)),
										h("td", { key: "at", style: cellStyle }, call.at.slice(11, 19)),
										h("td", { key: "m", style: cellStyle }, call.model ?? "未选"),
										h("td", { key: "msg", style: cellStyle }, String(call.messages)),
										h("td", { key: "t", style: cellStyle }, call.tools ? String(call.tools) : "—"),
										h(
											"td",
											{ key: "s", style: cellStyle, title: call.error ?? "" },
											`${LLM_STATUS_LABELS[call.status] ?? call.status}${call.error ? `：${call.error.slice(0, 60)}` : ""}`,
										),
										h("td", { key: "ms", style: cellStyle }, call.ms != null ? `${(call.ms / 1000).toFixed(1)}s` : "—"),
										h(
											"td",
											{ key: "u", style: cellStyle },
											call.usage ? `${call.usage.input_tokens} / ${call.usage.output_tokens}` : "—",
										),
										h("td", { key: "tc", style: cellStyle }, call.toolCalls ? String(call.toolCalls) : "—"),
									]),
								),
					),
				]),
				section("ct", "最近的适配器调用"),
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
				section("lt", "StoryForge 会话生命周期"),
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
