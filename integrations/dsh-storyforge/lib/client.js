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
 *   使新会话默认停在「对话」视图（开场白在那里），并自动打开右栏 StoryForge 面板。
 *
 * 产物是手写的 CJS 工厂包（宿主只吃已构建的 lib/client.js，不做编译）。
 *
 * 首页（左侧栏 StoryForge）：新建故事、我的故事、管线设置（底层模型 / 生成模式 /
 * 工具协议）、最近的收尾、底层模型调用；调试信息默认折叠。样式用 DSH 主题变量
 * （`--dsw-alias-*`），亮 / 暗主题自动跟随。
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
		/** DSH 自带的「对话」视图 id（`ui-conversation` 的 DEFAULT_VIEW_ID）。 */
		const CHAT_VIEW_ID = "chat";
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

		const postJson = (path, body) =>
			api(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

		const errorText = (e) => String(e?.message ?? e);

		/** 定时拉取 `/status`（sidecar 状态、收尾报告、底层调用、故事登记）；组件卸载即停。 */
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

		/** DSH 的会话列表快照（标题、是否在跑）；拿不到时为 null。 */
		function useSessionList() {
			const list = rootCtx?.sessions?.list;
			return React.useSyncExternalStore(
				(onChange) => list?.subscribe?.(onChange) ?? (() => {}),
				() => list?.getSnapshot?.() ?? null,
			);
		}

		/** 有进行中的条目时每秒重绘一次（显示实时耗时）。 */
		function useNow(active) {
			const [now, setNow] = React.useState(() => Date.now());
			React.useEffect(() => {
				if (!active) return undefined;
				const timer = setInterval(() => setNow(Date.now()), 1000);
				return () => clearInterval(timer);
			}, [active]);
			return active ? Math.max(now, Date.now()) : now;
		}

		//#region 格式化
		const pad = (n) => String(n).padStart(2, "0");

		/** 本地时间：今天只显示时分秒，其它日期带月-日。 */
		function formatTime(iso) {
			if (!iso) return "—";
			const d = new Date(iso);
			if (Number.isNaN(d.getTime())) return String(iso);
			const time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
			return d.toDateString() === new Date().toDateString() ? time : `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${time}`;
		}

		function formatFullTime(iso) {
			const d = new Date(iso);
			return Number.isNaN(d.getTime()) ? String(iso ?? "") : d.toLocaleString("zh-CN", { hour12: false });
		}

		/** 列表用的短时间：今天「今天 14:32」，昨天「昨天 14:32」，更早「10-09 14:32」。 */
		function formatShortTime(value) {
			const d = new Date(value);
			if (!value || Number.isNaN(d.getTime())) return "—";
			const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
			const today = new Date();
			if (d.toDateString() === today.toDateString()) return `今天 ${time}`;
			const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
			if (d.toDateString() === yesterday.toDateString()) return `昨天 ${time}`;
			return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${time}`;
		}

		function formatDuration(ms) {
			if (ms == null || Number.isNaN(ms)) return "—";
			if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
			const seconds = ms / 1000;
			if (seconds < 60) return `${seconds.toFixed(1)}s`;
			const minutes = Math.floor(seconds / 60);
			return `${minutes}m${pad(Math.floor(seconds - minutes * 60))}s`;
		}

		function formatTokens(n) {
			if (n == null) return "—";
			if (n < 1000) return String(n);
			if (n < 100_000) return `${(n / 1000).toFixed(1)}k`;
			if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
			return `${(n / 1_000_000).toFixed(1)}M`;
		}

		/** sidecar 的 UsageTotals → 一行文字。 */
		function describeUsage(usage) {
			if (!usage?.calls) return "—";
			const failed = usage.failed_calls ? `，失败 ${usage.failed_calls}` : "";
			const cached = usage.cache_read_tokens ? ` · 缓存 ${formatTokens(usage.cache_read_tokens)}` : "";
			return `${usage.calls} 次${failed} · 入 ${formatTokens(usage.input_tokens)} 出 ${formatTokens(usage.output_tokens)}${cached}`;
		}

		const shortId = (id) => (id ? String(id).slice(0, 8) : "—");
		//#endregion

		//#region 状态 → 文案与色调
		const ACCEPT = {
			accepted: ["已接受", "ok"],
			accepted_degraded: ["已接受（质量降级）", "warn"],
			failed: ["接受失败", "err"],
		};
		const POSTPROCESS = {
			PostProcessDone: ["后处理完成", "ok"],
			PostProcessFailed: ["后处理失败", "err"],
			PostProcessSkipped: ["后处理跳过", "idle"],
		};
		const LLM_STATUS = {
			running: ["进行中", "run"],
			ok: ["完成", "ok"],
			error: ["失败", "err"],
			aborted: ["已中止", "idle"],
		};
		const EXTRACTION = { extracted: "角色已识别", fallback: "角色识别失败（用兜底定义）" };

		function describeSettle(report) {
			if (!report) return "—";
			const parts = [POSTPROCESS[report.postprocess]?.[0] ?? "后处理无终态", ACCEPT[report.accept]?.[0] ?? report.accept];
			if (report.accept_error) parts.push(report.accept_error);
			if (report.usage?.calls) parts.push(`后处理 ${describeUsage(report.usage)}`);
			return parts.join(" · ");
		}

		function Badge(props) {
			return h("span", { className: `sf-badge sf-tone-${props.tone ?? "idle"}`, title: props.title }, props.children);
		}

		function AcceptBadge(props) {
			const [label, tone] = ACCEPT[props.report?.accept] ?? [props.report?.accept ?? "—", "idle"];
			return h(Badge, { tone, title: props.report?.accept_error ?? undefined }, label);
		}

		/** 写作引擎（sidecar 进程）整体状态。 */
		function bridgeState(status) {
			if (!status) return { label: "连接中…", tone: "idle" };
			if (status.pipeline === "fake") return { label: "假管线", tone: "idle" };
			if (status.statusError) return { label: "写作引擎异常", tone: "err", title: status.statusError };
			if (!status.bridge?.running) return { label: "写作引擎待命", tone: "idle", title: "sidecar 未运行；新建故事或写作时自动启动" };
			if (status.status?.busy) return { label: "写作引擎工作中", tone: "run", title: "正在写作或后台收尾" };
			return { label: "写作引擎就绪", tone: "ok", title: status.bridge.pid ? `sidecar pid ${status.bridge.pid}` : undefined };
		}

		/**
		 * 某个存档在写作引擎里的阶段：`queued`（排队等上一轮收尾或别的故事）、`writing`（前台成文）、
		 * `settling`（后台收尾：摘要 / 后处理 / 自动接受），都不是则 null。旧 sidecar 没有
		 * writing / queued 字段时退回「锁被占且是当前存档 = 收尾中」。
		 */
		function enginePhase(status, campaignId) {
			const engine = status?.status;
			if (!campaignId || !engine) return null;
			if (engine.writing === campaignId) return "writing";
			if ((engine.queued ?? []).includes(campaignId)) return "queued";
			if (engine.busy && !engine.writing && engine.active_campaign === campaignId) return "settling";
			return null;
		}
		//#endregion

		//#region 故事登记（插件 sessions.json + DSH 会话列表）
		/**
		 * 插件登记的故事，合上 DSH 会话列表的标题 / 运行状态和最近一次收尾，按最近活动排序。
		 * 标题优先用 DSH 侧显示的标题（用户在侧栏看到的就是它）。
		 */
		function buildStories(status, snapshot) {
			const registry = status?.sessions ?? {};
			const settles = status?.settles ?? [];
			// 会话列表还没加载（空）时不判"缺失"，免得一闪而过全部标成不在列表。
			const loaded = Boolean(snapshot?.byId) && Object.keys(snapshot.byId).length > 0;
			const stories = Object.entries(registry).map(([id, story]) => {
				const summary = snapshot?.byId?.[id];
				// 本次运行里的收尾优先，其次是登记里落盘的最近一次（跨 DSH 重启保留）。
				const live = story.campaignId ? settles.find((report) => report.campaign_id === story.campaignId) : undefined;
				const last = live ?? story.lastSettle;
				// 最近活动：DSH 会话的最后一条消息、最近一次收尾、建档时间，取最晚的（毫秒）。
				const activity = Math.max(
					typeof summary?.updatedAt === "number" ? summary.updatedAt : 0,
					Date.parse(last?.at ?? "") || 0,
					Date.parse(story.createdAt ?? "") || 0,
				);
				return {
					id,
					...story,
					displayTitle: summary?.displayTitle || story.title || id,
					running: Boolean(summary?.running),
					missing: loaded && !summary,
					last,
					activity,
				};
			});
			// 分叉子会话在第一次发送前还没有存档，但来源会话有存档就能建立分支。
			const byId = Object.fromEntries(stories.map((story) => [story.id, story]));
			for (const story of stories) {
				story.linked = Boolean(story.campaignId || (story.forkedFrom && byId[story.forkedFrom]?.campaignId));
			}
			stories.sort((a, b) => Number(b.linked) - Number(a.linked) || b.activity - a.activity);
			return stories;
		}

		function campaignTitles(stories) {
			const titles = {};
			for (const story of stories) if (story.campaignId && !titles[story.campaignId]) titles[story.campaignId] = story.displayTitle;
			return titles;
		}
		//#endregion

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

		function openPanel() {
			try {
				rootCtx.sidebarRight.openTab(PANEL_KIND);
			} catch (e) {
				console.error("[storyforge] open panel failed", e);
			}
		}

		function openSession(sessionId) {
			try {
				rootCtx.uiWorkspace.openSession(sessionId);
			} catch (e) {
				console.error("[storyforge] open session failed", e);
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
				{ type: "button", className: "sf-btn sf-btn-sm", title: "打开 StoryForge 面板", onClick: openPanel },
				"StoryForge 面板",
			);
		}

		/** 本故事的信息与最近几轮的收尾（右栏面板与主区视图共用）。 */
		function StoryDetails(props) {
			const { sessionId, status, snapshot, showTitle } = props;
			const stories = buildStories(status, snapshot);
			const story = stories.find((entry) => entry.id === sessionId);
			const parent = story?.forkedFrom ? stories.find((entry) => entry.id === story.forkedFrom) : undefined;
			const live = (status?.settles ?? []).filter((report) => story?.campaignId && report.campaign_id === story.campaignId);
			// 本次运行还没有收尾时，退回登记里落盘的最近一次。
			const settles = live.length > 0 ? live : story?.lastSettle ? [story.lastSettle] : [];
			const phase = enginePhase(status, story?.campaignId);
			const kv = (label, value, title) => [h("dt", { key: `${label}-k` }, label), h("dd", { key: `${label}-v`, title }, value ?? "—")];
			return h(
				React.Fragment,
				null,
				h(
					"div",
					{ className: "sf-panel-head", title: `会话 ${sessionId}` },
					showTitle && h("div", { className: "sf-card-title" }, story?.displayTitle ?? "StoryForge"),
					phase === "queued"
						? h(Badge, { tone: "run", title: "写作引擎正忙（上一轮在后台收尾，或另一个故事正在写），空出来后自动开始" }, "排队中")
						: story?.running || phase === "writing"
							? h(Badge, { tone: "run" }, "写作中")
							: phase === "settling"
								? h(Badge, { tone: "run" }, "后台收尾中")
								: settles[0] && h(AcceptBadge, { report: settles[0] }),
				),
				story
					? h(
							"dl",
							{ className: "sf-kv" },
							kv("角色卡", story.cardName ?? story.cardFile, story.cardFile),
							story.turns != null && kv("进度", story.turns === 0 ? "还没开始写" : `已写 ${story.turns} 轮`),
							story.forkedFrom && kv("分叉自", `${parent?.displayTitle ?? shortId(story.forkedFrom)}${story.forkTurns != null ? ` · 第 ${story.forkTurns} 轮` : ""}`),
							kv("存档", story.campaignId ? shortId(story.campaignId) : story.linked ? "第一次发送时建立分支" : "未关联", story.campaignId),
							story.extractionStatus && kv("角色识别", EXTRACTION[story.extractionStatus] ?? story.extractionStatus),
							story.extractionError && kv("识别失败原因", story.extractionError, story.extractionErrorDetail),
						)
					: h("div", { className: "sf-muted" }, "未登记的 StoryForge 会话（例如从新会话页直接选了预设）。请从 StoryForge 首页新建故事。"),
				h("div", { className: "sf-section-title" }, "最近几轮的收尾"),
				settles.length === 0
					? h("div", { className: "sf-muted" }, "写完一轮后显示后处理和接受的结果")
					: h(
							"ul",
							{ className: "sf-settle-list" },
							settles.map((report, i) =>
								h(
									"li",
									{ key: `${report.at}-${i}` },
									h("span", { className: "sf-muted sf-nowrap", title: formatFullTime(report.at) }, formatShortTime(report.at)),
									h(AcceptBadge, { report }),
									report.usage?.calls
										? h("span", { className: "sf-muted" }, `后处理 ${describeUsage(report.usage)}`)
										: h("span", { className: "sf-muted" }, POSTPROCESS[report.postprocess]?.[0] ?? ""),
								),
							),
						),
			);
		}

		/** 主区 StoryForge 视图：本故事概况；卡内状态在右栏面板，卡片壳留到后续阶段。 */
		function StoryForgeView(props) {
			const sessionId = props.sfSessionId ?? props.sessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);
			const status = useBridgeStatus(isStoryForge);
			const snapshot = useSessionList();
			if (!isStoryForge) return h("div", { className: "sf-root sf-view sf-muted" }, "此视图仅用于 StoryForge 会话。");
			return h(
				"div",
				{ className: "sf-root sf-view" },
				h(StoryDetails, { sessionId, status, snapshot, showTitle: true }),
				h(
					"p",
					{ className: "sf-muted sf-small" },
					"正文在「对话」标签：思考块里是导演、角色、执笔的过程，回复是成文。卡内状态栏和按钮在右栏 StoryForge 面板；卡片壳（卡自带的网页界面）留到后续阶段。",
				),
				h("div", null, h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: openPanel }, "在右栏打开")),
			);
		}

		//#region 卡内状态面板（MVU 原生渲染 + 交互按钮）
		/**
		 * 每个 StoryForge 会话的输入框（停靠区状态条在本会话挂载时登记）。交互按钮的「下一轮
		 * 提示」经它发出——就是用户自己发了一条消息，DSH 照常记一轮。
		 */
		const composers = new Map();

		/** 输入框里写入 `text` 并发送；输入框不在、正忙或有草稿时不动它，返回原因。 */
		async function sendViaComposer(sessionId, text) {
			const composer = composers.get(sessionId);
			if (!composer) return "no-composer";
			const before = composer.read();
			if (before.phase !== "plain") return "busy";
			if (before.draft.trim()) return "draft";
			composer.actions.setDraft(text);
			// 编辑器异步落草稿：等输入状态里出现这段文字再发送，免得发出空消息。
			for (let i = 0; i < 20 && composer.read().draft.trim() !== text.trim(); i++) {
				await new Promise((resolve) => setTimeout(resolve, 50));
			}
			if (composer.read().draft.trim() !== text.trim()) return "busy";
			composer.actions.submit();
			return "sent";
		}

		const MVU_NOT_SENT = {
			"no-composer": "输入框不在当前页面",
			busy: "输入框正忙",
			draft: "输入框里有没发出去的草稿",
		};

		/**
		 * 面板数据：挂载、换会话、本存档有新的收尾（后处理写回变量）时重取。
		 * `refreshKey` 由调用方拼（存档 id + 最近一次收尾时间 + 轮数）。
		 */
		function useMvuModel(sessionId, enabled, refreshKey) {
			const [state, setState] = React.useState({ model: null, error: null, loading: false });
			const seq = React.useRef(0);
			const load = React.useCallback(() => {
				const current = ++seq.current;
				setState((prev) => ({ ...prev, loading: true }));
				return postJson("/mvu", { sessionId })
					.then((body) => current === seq.current && setState({ model: body.model, error: null, loading: false }))
					.catch((e) => current === seq.current && setState({ model: null, error: errorText(e), loading: false }));
			}, [sessionId]);
			React.useEffect(() => {
				if (!enabled || !sessionId) return;
				void load();
			}, [enabled, sessionId, refreshKey, load]);
			const replace = React.useCallback((model) => {
				seq.current++;
				setState({ model, error: null, loading: false });
			}, []);
			return [state, load, replace];
		}

		function MvuRow(props) {
			const { row } = props;
			if (row.kind === "bar") {
				return h(
					"div",
					{ className: "sf-mvu-row", title: row.element },
					h("span", { className: "sf-mvu-label" }, row.label),
					h(
						"span",
						{ className: `sf-mvu-bar sf-tone-${row.tone}`, role: "progressbar", "aria-valuenow": row.value ?? 0, "aria-valuemax": row.max },
						h("span", { className: "sf-mvu-fill", style: { width: `${row.percent}%` } }),
					),
					h("span", { className: "sf-mvu-value" }, `${row.value ?? "—"} / ${row.max}`),
				);
			}
			return h(
				"div",
				{ className: "sf-mvu-row", title: row.element },
				h("span", { className: "sf-mvu-label" }, row.label),
				row.kind === "tag"
					? h("span", { className: "sf-mvu-tag" }, row.text)
					: h("span", { className: `sf-mvu-text${row.kind === "icon" ? " sf-mvu-icon" : ""}` }, row.text),
			);
		}

		/** 右栏里的卡内状态：分析 / 应用变量定义 / 状态行 / 交互按钮。 */
		function MvuPanel(props) {
			const { sessionId, status, snapshot } = props;
			const story = status?.sessions?.[sessionId];
			const campaignId = story?.campaignId ?? null;
			const lastSettle = (status?.settles ?? []).find((report) => campaignId && report.campaign_id === campaignId);
			const refreshKey = `${campaignId}|${lastSettle?.at ?? story?.lastSettle?.at ?? ""}|${story?.turns ?? ""}`;
			const [{ model, error, loading }, reload, replace] = useMvuModel(sessionId, Boolean(story), refreshKey);
			const [action, setAction] = React.useState(null);
			const [notice, setNotice] = React.useState(null);
			const elapsed = useElapsed(action === "analyze");
			const phase = enginePhase(status, campaignId);
			const running = Boolean(snapshot?.byId?.[sessionId]?.running);
			const engineBusy = Boolean(phase) || running;
			const disabled = Boolean(action) || engineBusy;
			const busyTitle = engineBusy ? "写作或后台收尾结束后再操作" : undefined;

			const run = async (name, work) => {
				setAction(name);
				setNotice(null);
				try {
					await work();
				} catch (e) {
					setNotice({ tone: "err", text: errorText(e) });
					if (/刷新面板/.test(errorText(e))) void reload();
				} finally {
					setAction(null);
				}
			};

			const analyze = () =>
				run("analyze", async () => {
					const result = await postJson("/mvu/analyze", { sessionId });
					replace(result.model);
					const cost = result.usage?.calls ? `（${describeUsage(result.usage)}）` : "";
					if (result.empty) {
						setNotice({
							tone: "warn",
							text: result.error
								? `分析没拿到状态栏规则：${result.error}${cost}`
								: `分析没找到可原生渲染的状态栏或交互按钮，只得到变量字段${cost}。`,
						});
					} else {
						setNotice({ tone: "ok", text: `分析完成${cost}` });
					}
				});

			const applySchema = () =>
				run("apply", async () => {
					const result = await postJson("/mvu/apply", { sessionId });
					replace(result.model);
					setNotice({ tone: "ok", text: "变量定义已应用到角色，已有角色补上了默认值。" });
				});

			const interact = (item) =>
				run(`interact-${item.index}`, async () => {
					const result = await postJson("/mvu/interact", { sessionId, index: item.index, label: item.label });
					replace(result.model);
					if (result.failed > 0) {
						const detail = result.results
							.filter((entry) => !entry.ok)
							.map((entry) => `${entry.key}：${entry.error}`)
							.join("\n");
						setNotice({ tone: "err", text: `「${result.label}」有 ${result.failed} 个变量没写进去，没有发送下一轮提示：\n${detail}` });
						return;
					}
					// 卡规则里解释不了的动作（例如带函数的取值表达式）不执行，照实说明。
					const skipped = (result.skipped ?? [])
						.map((entry) => `${entry.key ?? entry.kind}：${entry.reason}`)
						.join("；");
					const updated = result.results.length > 0 ? "已更新变量" : "没有改变量";
					const head = `「${result.label}」${updated}${skipped ? `（未执行：${skipped}）` : ""}`;
					const tone = skipped ? "warn" : "ok";
					const hint = result.hints.join("\n");
					if (!hint) {
						setNotice({ tone, text: `${head}。` });
						return;
					}
					const sent = await sendViaComposer(sessionId, hint);
					setNotice(
						sent === "sent"
							? { tone, text: `${head}，并发送了：${hint}` }
							: { tone: "warn", text: `${head}。下一轮提示没有自动发送（${MVU_NOT_SENT[sent]}），需要的话自己发：`, hint },
					);
				});

			if (!story) return null;
			const head = h(
				"div",
				{ className: "sf-panel-head" },
				h("div", { className: "sf-section-title" }, "卡内状态"),
				model?.state === "ready" &&
					h(
						"span",
						{ className: "sf-muted sf-small", title: model.analyzedAt ? `分析于 ${formatFullTime(model.analyzedAt)}` : undefined },
						`${model.bindingCount} 项状态 · ${model.interactions.length} 个按钮 · ${model.ruleCount} 条更新规则`,
					),
				h("span", { className: "sf-spacer" }),
				h(
					"button",
					{ type: "button", className: "sf-btn sf-btn-sm", disabled: loading || Boolean(action), onClick: () => void reload(), title: "重新读取变量" },
					"刷新",
				),
			);
			const noticeView =
				notice &&
				h(
					"div",
					{ className: `sf-notice sf-notice-${notice.tone === "err" ? "warn" : notice.tone}`, role: notice.tone === "err" ? "alert" : "status" },
					h(
						"div",
						{ className: "sf-grow" },
						notice.text,
						notice.hint && h("div", { className: "sf-mono sf-gap-top" }, notice.hint),
					),
					notice.hint &&
						navigator.clipboard &&
						h(
							"button",
							{ type: "button", className: "sf-btn sf-btn-sm", onClick: () => void navigator.clipboard.writeText(notice.hint) },
							"复制",
						),
					h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setNotice(null) }, "知道了"),
				);
			const body = (() => {
				if (error) return h("div", { className: "sf-error" }, error);
				if (!model) return h("div", { className: "sf-muted sf-small" }, "读取中…");
				if (model.state === "none") {
					return h(
						"div",
						{ className: "sf-muted sf-small" },
						model.reason === "unlinked" ? "这个分叉还没有存档，第一次发送后显示。" : "当前管线没有卡内状态。",
					);
				}
				const analyzeButton = h(
					"button",
					{ type: "button", className: "sf-btn sf-btn-sm", disabled, title: busyTitle, onClick: analyze },
					action === "analyze" ? `分析中… ${elapsed}s` : model.state === "ready" ? "重新分析" : "分析变量规则",
				);
				if (model.state === "unanalyzed") {
					return h(
						React.Fragment,
						null,
						h(
							"div",
							{ className: "sf-muted sf-small" },
							"这张卡还没分析过状态栏和变量规则。分析会调用一次底层模型（大卡要几十秒），之后状态栏和卡里的按钮在这里原生显示。",
						),
						h("div", null, analyzeButton),
					);
				}
				const pending = model.pendingApply ?? [];
				return h(
					React.Fragment,
					null,
					pending.length > 0 &&
						h(
							"div",
							{ className: "sf-notice sf-notice-warn" },
							h(
								"div",
								{ className: "sf-grow" },
								"分析出的变量定义还没应用到角色：",
								pending.map((entry) =>
									h(
										"div",
										{ key: entry.definition_id, className: "sf-small" },
										`${entry.definition_name || shortId(entry.definition_id)}：新增 ${entry.added.length} 项${entry.overwritten.length ? `，改写 ${entry.overwritten.length} 项` : ""}`,
									),
								),
							),
							h(
								"button",
								{ type: "button", className: "sf-btn sf-btn-sm sf-btn-primary", disabled, title: busyTitle, onClick: applySchema },
								action === "apply" ? "应用中…" : "应用到角色",
							),
						),
					model.sections.length === 0
						? h(
								"div",
								{ className: "sf-muted sf-small" },
								model.bindingCount === 0
									? "分析没找到可原生显示的状态项。"
									: "还没有绑定这张卡角色定义的角色，状态项暂时没有可显示的对象。",
							)
						: model.sections.map((section) =>
								h(
									"div",
									{ key: section.instanceId, className: "sf-mvu-section" },
									model.sections.length > 1 && h("div", { className: "sf-mvu-name" }, section.instanceName || shortId(section.instanceId)),
									section.rows.map((row, i) => h(MvuRow, { key: `${row.element}-${i}`, row })),
								),
							),
					model.interactions.length > 0 &&
						h(
							"div",
							{ className: "sf-mvu-actions" },
							model.interactions.map((item) =>
								h(
									"button",
									{ key: item.index, type: "button", className: "sf-btn sf-btn-sm", disabled, title: busyTitle, onClick: () => interact(item) },
									action === `interact-${item.index}` ? `${item.label}…` : item.label,
								),
							),
						),
					model.fallbacks.length > 0 &&
						h(
							"details",
							{ className: "sf-mvu-fallbacks" },
							h("summary", { className: "sf-muted sf-small" }, `${model.fallbacks.length} 处需要卡内脚本，DSH 里不执行`),
							h(
								"ul",
								{ className: "sf-plain-list sf-small" },
								model.fallbacks.map((fragment, i) =>
									h("li", { key: i }, fragment.description || "（无说明）", fragment.reason && h("span", { className: "sf-muted" }, ` · ${fragment.reason}`)),
								),
							),
						),
					h("div", null, analyzeButton),
				);
			})();
			return h("div", { className: "sf-mvu" }, head, noticeView, body);
		}
		//#endregion

		/** 右栏 StoryForge 面板。 */
		function PanelTab(props) {
			const sessionId = props.sessionId ?? rootCtx.sidebarRight?.mounted?.getSnapshot?.();
			const isStoryForge = useIsStoryForge(props, sessionId);
			const status = useBridgeStatus(isStoryForge);
			const snapshot = useSessionList();
			if (!isStoryForge) return h("div", { className: "sf-root sf-panel sf-muted" }, "此面板仅用于 StoryForge 会话。");
			return h(
				"div",
				{ className: "sf-root sf-panel" },
				h(StoryDetails, { sessionId, status, snapshot, showTitle: true }),
				h(MvuPanel, { key: sessionId, sessionId, status, snapshot }),
			);
		}

		/** 输入框模型位的只读标签（只在 StoryForge 会话里注册）。 */
		function ModelLock() {
			return h(
				"span",
				{
					style: {
						display: "var(--dsh-composer-model-text-display, inline)",
						whiteSpace: "nowrap",
						padding: "0 6px",
						fontSize: 12,
						color: "var(--dsw-alias-label-tertiary, inherit)",
					},
					title: "StoryForge 会话的模型固定为 StoryForge 管线",
				},
				"StoryForge 管线",
			);
		}

		/** 停靠区状态条（输入框下方）。顺带登记本会话的输入框，供右栏交互按钮发下一轮提示。 */
		function DockStatus(props) {
			const sessionId = props.sfSessionId ?? props.sessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);
			const status = useBridgeStatus(isStoryForge);
			const input = typeof props.useInput === "function" ? props.useInput((state) => state) : null;
			const inputRef = React.useRef(input);
			inputRef.current = input;
			const inputActions = props.inputActions;
			React.useEffect(() => {
				if (!isStoryForge || !sessionId || !inputActions) return undefined;
				const entry = {
					actions: inputActions,
					read: () => ({ draft: inputRef.current?.draft ?? "", phase: inputRef.current?.phase ?? "plain" }),
				};
				composers.set(sessionId, entry);
				return () => {
					if (composers.get(sessionId) === entry) composers.delete(sessionId);
				};
			}, [isStoryForge, sessionId, inputActions]);
			if (!isStoryForge) return null;
			if (status?.pipeline === "fake") return h("div", { className: "sf-root sf-dock" }, "StoryForge · 假管线");
			const story = status?.sessions?.[sessionId];
			const model = status?.model ? status.model.model : "未选底层模型";
			const phase = enginePhase(status, story?.campaignId);
			const state = !status?.bridge?.running
				? "引擎待命"
				: { queued: "排队中", writing: "写作中", settling: "后台收尾中" }[phase] ?? "空闲";
			const last =
				(status?.settles ?? []).find((report) => story?.campaignId && report.campaign_id === story.campaignId) ?? story?.lastSettle;
			return h(
				"div",
				{ className: "sf-root sf-dock", title: status?.model ? `${status.model.provider}/${status.model.model}` : undefined },
				`StoryForge · ${model} · ${state}${last ? ` · 上一轮（${formatTime(last.at)}）：${describeSettle(last)}` : ""}`,
			);
		}

		//#region 首页
		function Card(props) {
			return h(
				"section",
				{ className: `sf-card ${props.className ?? ""}` },
				h(
					"div",
					{ className: "sf-card-head" },
					h("div", { className: "sf-card-title" }, props.title),
					props.sub && h("div", { className: "sf-card-sub" }, props.sub),
					props.actions && h("div", { className: "sf-card-actions" }, props.actions),
				),
				props.children,
			);
		}

		/** 表格列：`{ key, label, num?, render(row) }`。 */
		function Table(props) {
			const { columns, rows, rowKey, empty } = props;
			if (rows.length === 0) return h("div", { className: "sf-empty" }, empty);
			return h(
				"div",
				{ className: "sf-table-wrap" },
				h(
					"table",
					{ className: "sf-table" },
					h(
						"thead",
						null,
						h(
							"tr",
							null,
							columns.map((column) => h("th", { key: column.key, className: column.num ? "sf-num" : undefined }, column.label)),
						),
					),
					h(
						"tbody",
						null,
						rows.map((row, i) =>
							h(
								"tr",
								{ key: rowKey(row, i) },
								columns.map((column) =>
									h("td", { key: column.key, className: column.num ? "sf-num" : column.className }, column.render(row)),
								),
							),
						),
					),
				),
			);
		}

		/** 列表默认只显示前 N 条，多出来的给一个展开按钮。 */
		function useCollapsed(rows, limit) {
			const [expanded, setExpanded] = React.useState(false);
			const shown = expanded ? rows : rows.slice(0, limit);
			const toggle =
				rows.length > limit &&
				h(
					"button",
					{ type: "button", className: "sf-btn sf-btn-sm sf-more", onClick: () => setExpanded((value) => !value) },
					expanded ? "收起" : `显示全部 ${rows.length} 条`,
				);
			return [shown, toggle];
		}

		/** 进行中的操作计时（秒），busy 为假时归零。 */
		function useElapsed(busy) {
			const [seconds, setSeconds] = React.useState(0);
			React.useEffect(() => {
				if (!busy) {
					setSeconds(0);
					return undefined;
				}
				const started = Date.now();
				const timer = setInterval(() => setSeconds(Math.floor((Date.now() - started) / 1000)), 1000);
				return () => clearInterval(timer);
			}, [busy]);
			return seconds;
		}

		/** 打开新建的故事：先写会话状态（默认停在「对话」看开场白）并约定自动打开右栏。 */
		function openNewStory(sessionId) {
			localStorage.setItem(
				`${CONVERSATION_STORE_KEY}.${sessionId}`,
				JSON.stringify({ draft: "", view: CHAT_VIEW_ID, viewRequest: null }),
			);
			localStorage.setItem(AUTO_PANEL_KEY + sessionId, "1");
			openSession(sessionId);
		}

		const cardLabel = (entry) => (entry.name && entry.name !== entry.file ? `${entry.name}（${entry.file}）` : entry.file);

		function NewStoryCard(props) {
			const { cards, fake, modelReady, onCreated, onCardsChanged, onError } = props;
			const [card, setCard] = React.useState("");
			const [title, setTitle] = React.useState("");
			const [busy, setBusy] = React.useState(false);
			const [importing, setImporting] = React.useState(false);
			const [notice, setNotice] = React.useState(null);
			const fileInput = React.useRef(null);
			const elapsed = useElapsed(busy);
			const files = cards?.cards ?? [];
			React.useEffect(() => {
				if (!files.some((entry) => entry.file === card) && files[0]) setCard(files[0].file);
			}, [card, files]);

			const create = async () => {
				setBusy(true);
				setNotice(null);
				onError(null);
				try {
					const { sessionId, warning } = await postJson("/sessions", { title: title.trim() || undefined, card });
					setTitle("");
					onCreated();
					// 识别失败（多半是底层模型没配好）时先留在首页说明原因，由用户决定是否打开。
					if (warning) setNotice({ tone: "warn", text: warning, sessionId });
					else openNewStory(sessionId);
				} catch (e) {
					onError(errorText(e));
				} finally {
					setBusy(false);
				}
			};

			const importCard = async (file) => {
				if (!file) return;
				setImporting(true);
				setNotice(null);
				onError(null);
				try {
					const { card: saved } = await api(`/cards/import?name=${encodeURIComponent(file.name)}`, {
						method: "POST",
						headers: { "content-type": "application/octet-stream" },
						body: file,
					});
					await onCardsChanged();
					setCard(saved.file);
					setNotice({ tone: "ok", text: `已导入「${saved.name}」（${saved.file}）` });
				} catch (e) {
					onError(`导入角色卡失败：${errorText(e)}`);
				} finally {
					setImporting(false);
					if (fileInput.current) fileInput.current.value = "";
				}
			};

			const blocker = fake ? null : !modelReady ? "先在「管线设置」里选底层模型" : !card ? "先导入一张角色卡" : null;
			return h(
				Card,
				{ title: "新建故事", sub: fake ? "假管线模式" : `${files.length} 张角色卡可选` },
				h(
					"div",
					{ className: "sf-form" },
					!fake && h("label", { htmlFor: "sf-card" }, "角色卡"),
					!fake &&
						h(
							"div",
							{ className: "sf-row" },
							h(
								"select",
								{ id: "sf-card", className: "sf-input", value: card, onChange: (e) => setCard(e.target.value) },
								files.length === 0
									? h("option", { value: "" }, "（还没有角色卡）")
									: files.map((entry) => h("option", { key: entry.file, value: entry.file }, cardLabel(entry))),
							),
							h("input", {
								ref: fileInput,
								type: "file",
								accept: ".png,.json,image/png,application/json",
								hidden: true,
								onChange: (e) => void importCard(e.target.files?.[0]),
							}),
							h(
								"button",
								{
									type: "button",
									className: "sf-btn",
									disabled: importing || busy,
									title: "导入 SillyTavern 角色卡（.png / .json）",
									onClick: () => fileInput.current?.click(),
								},
								importing ? "导入中…" : "导入角色卡…",
							),
						),
					h("label", { htmlFor: "sf-title" }, "标题"),
					h("input", {
						id: "sf-title",
						className: "sf-input",
						value: title,
						placeholder: "可空，默认用卡名",
						onChange: (e) => setTitle(e.target.value),
						onKeyDown: (e) => {
							if (e.key === "Enter" && !busy && !blocker) void create();
						},
					}),
				),
				h(
					"div",
					{ className: "sf-actions" },
					h(
						"button",
						{ type: "button", className: "sf-btn sf-btn-primary", disabled: busy || Boolean(blocker), onClick: () => void create() },
						busy ? (fake ? "创建中…" : `导卡、识别角色中… ${elapsed} 秒`) : "新建故事",
					),
					blocker
						? h("span", { className: "sf-muted sf-small" }, blocker)
						: busy && !fake && h("span", { className: "sf-muted sf-small" }, "识别角色要调一次底层模型，可能需要一两分钟"),
				),
				notice &&
					h(
						"div",
						{ className: `sf-notice sf-notice-${notice.tone}` },
						h("span", { className: "sf-grow" }, notice.text),
						notice.sessionId &&
							h(
								"button",
								{
									type: "button",
									className: "sf-btn sf-btn-sm",
									onClick: () => {
										const { sessionId } = notice;
										setNotice(null);
										openNewStory(sessionId);
									},
								},
								"仍然打开故事",
							),
						h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setNotice(null) }, "关闭"),
					),
				!fake &&
					cards?.cardsDir &&
					h("div", { className: "sf-muted sf-small sf-gap-top" }, `也可以直接把 .png / .json 角色卡放进 ${cards.cardsDir}，再点右上角「刷新」`),
			);
		}

		function SettingsCard(props) {
			const { models, onChange, onError } = props;
			const [saved, setSaved] = React.useState(null);
			React.useEffect(() => {
				if (!saved) return undefined;
				const timer = setTimeout(() => setSaved(null), 1600);
				return () => clearTimeout(timer);
			}, [saved]);

			/** 下拉框的选项与 { provider, model } 一一对应（value 用序号，避免拼接 id）。 */
			const choices = React.useMemo(
				() =>
					(models?.providers ?? []).flatMap((provider) =>
						provider.models.map((model) => ({ provider: provider.provider, model: model.id })),
					),
				[models],
			);

			const save = async (field, patch, local) => {
				try {
					await postJson("/settings", patch);
					onChange(local ?? patch);
					setSaved(field);
				} catch (e) {
					onError(errorText(e));
				}
			};

			const selected = models?.selected;
			const selectedIndex = selected
				? choices.findIndex((choice) => choice.provider === selected.provider && choice.model === selected.model)
				: -1;

			/** 「测试连接」：结果跟着当前选中的模型走，换模型就清掉。 */
			const selectedKey = selected ? `${selected.provider}/${selected.model}` : "";
			const [probe, setProbe] = React.useState(null);
			const testing = probe?.running === true;
			const probeElapsed = useElapsed(testing);
			React.useEffect(() => setProbe(null), [selectedKey]);
			const runProbe = async () => {
				setProbe({ running: true });
				try {
					const result = await postJson("/models/test", {});
					setProbe({ key: selectedKey, ...result });
				} catch (e) {
					setProbe({ key: selectedKey, passed: false, error: errorText(e) });
				}
			};
			const probeLine = testing
				? h("span", { className: "sf-muted sf-small" }, `正在请求 ${selectedKey}… ${probeElapsed} 秒`)
				: probe?.key === selectedKey &&
					(probe.passed
						? h("span", { className: "sf-probe-ok sf-small" }, `连接正常 · ${(probe.ms / 1000).toFixed(1)} 秒`)
						: h("span", { className: "sf-probe-err sf-small", title: probe.detail ?? "" }, probe.error));
			let optionIndex = 0;
			const savedMark = (field) => saved === field && h("span", { className: "sf-saved" }, "已保存");
			const field = (id, label, control, hint, key) => [
				h("label", { key: `${id}-l`, htmlFor: id }, label),
				h("div", { key: `${id}-c`, className: "sf-field" }, control, savedMark(key)),
				hint && h("div", { key: `${id}-h`, className: "sf-hint" }, hint),
			];
			const optionSelect = (id, labels, value, onSelect) =>
				h(
					"select",
					{ id, className: "sf-input", value: value ?? "", onChange: (e) => void onSelect(e.target.value) },
					Object.entries(labels ?? {}).map(([optionId, label]) => h("option", { key: optionId, value: optionId }, label)),
				);

			if (!models) return h(Card, { title: "管线设置" }, h("div", { className: "sf-muted" }, "加载中…"));
			return h(
				Card,
				{ title: "管线设置", sub: "只作用于 StoryForge 会话" },
				h(
					"div",
					{ className: "sf-form" },
					field(
						"sf-model",
						"底层模型",
						h(
							"select",
							{
								id: "sf-model",
								className: "sf-input",
								value: selectedIndex >= 0 ? String(selectedIndex) : "",
								onChange: (e) => {
									const choice = choices[Number(e.target.value)];
									if (choice) void save("model", choice, { selected: choice });
								},
							},
							h(
								"option",
								{ value: "", disabled: true },
								selected && selectedIndex < 0 ? `${selected.provider}/${selected.model}（已不可用）` : "选择底层模型…",
							),
							(models.providers ?? []).map((provider) =>
								h(
									"optgroup",
									{ key: provider.provider, label: provider.error ? `${provider.name}（${provider.error}）` : provider.name },
									provider.models.map((model) => h("option", { key: model.id, value: String(optionIndex++) }, model.name)),
								),
							),
						),
						"导演、角色、编剧、后处理共用；不影响 DSH 其它会话的模型",
						"model",
					),
					h("div", { key: "sf-probe-l" }),
					h(
						"div",
						{ key: "sf-probe-c", className: "sf-field sf-probe" },
						h(
							"button",
							{
								type: "button",
								className: "sf-btn sf-btn-sm",
								disabled: !selected || selectedIndex < 0 || testing,
								title: "用选中的模型发一条很短的请求，确认密钥与网络可用",
								onClick: () => void runProbe(),
							},
							"测试连接",
						),
						probeLine,
					),
					field(
						"sf-mode",
						"生成模式",
						optionSelect("sf-mode", models.generationModes, models.generationMode, (value) =>
							save("generationMode", { generationMode: value }),
						),
						"自动路由想升到高成本群像时会报错，需要在这里显式选群像",
						"generationMode",
					),
					field(
						"sf-tools",
						"工具协议",
						optionSelect("sf-tools", models.toolModes, models.toolMode, (value) => save("toolMode", { toolMode: value })),
						"模型不支持 function calling 时选文本降级",
						"toolMode",
					),
				),
			);
		}

		function StoriesCard(props) {
			const { stories, status } = props;
			const linkedStories = stories.filter((story) => story.linked);
			const unlinked = stories.filter((story) => !story.linked);
			const [shown, toggle] = useCollapsed(linkedStories, 8);
			const [showUnlinked, setShowUnlinked] = React.useState(false);
			const busyCampaign = status?.status?.busy ? status.status.active_campaign : null;
			const byId = Object.fromEntries(stories.map((story) => [story.id, story]));
			const renderStory = (story) => {
				const meta = [];
				if (story.turns != null) meta.push(story.turns === 0 ? "还没开始写" : `已写 ${story.turns} 轮`);
				if (story.cardName || story.cardFile) meta.push(story.cardName ?? story.cardFile);
				if (story.forkedFrom) {
					const parent = byId[story.forkedFrom];
					meta.push(`分叉自「${parent?.displayTitle ?? shortId(story.forkedFrom)}」${story.forkTurns != null ? `第 ${story.forkTurns} 轮` : ""}`);
				}
				if (!story.campaignId) meta.push(story.linked ? "第一次发送时建立存档分支" : "未关联存档");
				const state = story.running
					? h(Badge, { tone: "run" }, "写作中")
					: story.campaignId && busyCampaign === story.campaignId
						? h(Badge, { tone: "run" }, "后台收尾中")
						: story.missing
							? h(Badge, { tone: "idle", title: "DSH 会话列表里没有它（可能已归档）" }, "不在会话列表")
							: story.last && h(AcceptBadge, { report: story.last });
				// 整行可点（键盘 Enter / 空格同样打开）；DSH 列表里已没有的会话不可打开。
				const open = () => {
					if (!story.missing) openSession(story.id);
				};
				return h(
					"div",
					{
						key: story.id,
						className: `sf-story${story.campaignId ? "" : " sf-story-dim"}${story.missing ? " sf-story-missing" : ""}`,
						role: "button",
						tabIndex: story.missing ? -1 : 0,
						"aria-disabled": story.missing || undefined,
						onClick: open,
						onKeyDown: (e) => {
							if (e.key !== "Enter" && e.key !== " ") return;
							e.preventDefault();
							open();
						},
					},
					h(
						"div",
						{ className: "sf-story-main" },
						h("div", { className: "sf-story-title", title: story.displayTitle }, story.displayTitle),
						h("div", { className: "sf-story-meta", title: meta.join(" · ") }, meta.join(" · ") || "—"),
					),
					h(
						"div",
						{ className: "sf-story-side" },
						story.activity > 0 &&
							h(
								"span",
								{ className: "sf-muted sf-small sf-nowrap", title: `最近活动 ${formatFullTime(story.activity)}` },
								formatShortTime(story.activity),
							),
						state,
						!story.missing && h("span", { className: "sf-chevron", "aria-hidden": "true" }, "›"),
					),
				);
			};
			return h(
				Card,
				{ title: "我的故事", sub: linkedStories.length ? `${linkedStories.length} 个` : undefined },
				linkedStories.length === 0
					? h("div", { className: "sf-empty" }, "还没有故事。选一张角色卡新建一个。")
					: h("div", { className: "sf-stories" }, shown.map(renderStory)),
				toggle,
				unlinked.length > 0 &&
					h(
						"button",
						{ type: "button", className: "sf-btn sf-btn-sm sf-more", onClick: () => setShowUnlinked((value) => !value) },
						showUnlinked ? "收起未关联存档的会话" : `显示 ${unlinked.length} 个未关联存档的会话`,
					),
				showUnlinked && h("div", { className: "sf-stories sf-gap-top" }, unlinked.map(renderStory)),
			);
		}

		function SettlesCard(props) {
			const { settles, titles } = props;
			const [shown, toggle] = useCollapsed(settles, 6);
			return h(
				Card,
				{ title: "最近的收尾", sub: "每轮正文出来后，后台做后处理并自动接受" },
				h(Table, {
					rows: shown,
					rowKey: (report, i) => `${report.at}-${i}`,
					empty: "写完一轮后，这里会显示后处理和接受的结果",
					columns: [
						{
							key: "at",
							label: "时间",
							className: "sf-nowrap",
							render: (report) => h("span", { title: formatFullTime(report.at) }, formatTime(report.at)),
						},
						{
							key: "story",
							label: "故事",
							render: (report) => h("span", { title: report.campaign_id }, titles[report.campaign_id] ?? `存档 ${shortId(report.campaign_id)}`),
						},
						{ key: "accept", label: "接受", render: (report) => h(AcceptBadge, { report }) },
						{
							key: "post",
							label: "后处理",
							render: (report) => {
								const [label, tone] = POSTPROCESS[report.postprocess] ?? ["无终态", "idle"];
								return h(Badge, { tone, title: report.postprocess_detail ?? undefined }, label);
							},
						},
						{ key: "usage", label: "后处理用量", className: "sf-nowrap", render: (report) => describeUsage(report.usage) },
					],
				}),
				toggle,
			);
		}

		function LlmCallsCard(props) {
			const { calls, total } = props;
			const running = calls.some((call) => call.status === "running");
			const now = useNow(running);
			const [shown, toggle] = useCollapsed(calls, 12);
			const models = [...new Set(calls.map((call) => call.model ?? "未选"))];
			const count = (state) => calls.filter((call) => call.status === state).length;
			const sum = (field) => calls.reduce((acc, call) => acc + (call.usage?.[field] ?? 0), 0);
			const stats = [
				["共", `${total} 次`],
				["进行中", count("running")],
				["失败", count("error")],
				["中止", count("aborted")],
				["输入", formatTokens(sum("input_tokens"))],
				["输出", formatTokens(sum("output_tokens"))],
				["缓存读", formatTokens(sum("cache_read_tokens"))],
			];
			const columns = [
				{ key: "seq", label: "#", num: true, render: (call) => call.seq },
				{
					key: "at",
					label: "时间",
					className: "sf-nowrap",
					render: (call) => h("span", { title: formatFullTime(call.at) }, formatTime(call.at)),
				},
				{ key: "role", label: "角色", className: "sf-nowrap", render: (call) => call.role ?? "—" },
				{
					key: "status",
					label: "状态",
					render: (call) => {
						const [label, tone] = LLM_STATUS[call.status] ?? [call.status, "idle"];
						return h(
							React.Fragment,
							null,
							h(Badge, { tone, title: call.error ?? undefined }, label),
							call.error && h("div", { className: "sf-muted sf-small sf-error-line", title: call.error }, call.error),
						);
					},
				},
				{
					key: "ms",
					label: "耗时",
					num: true,
					render: (call) => formatDuration(call.status === "running" ? now - new Date(call.at).getTime() : call.ms),
				},
				{ key: "in", label: "输入", num: true, render: (call) => formatTokens(call.usage?.input_tokens) },
				{ key: "out", label: "输出", num: true, render: (call) => formatTokens(call.usage?.output_tokens) },
				{
					key: "tools",
					label: "工具 · 调用",
					num: true,
					render: (call) => (call.tools ? `${call.tools} · ${call.toolCalls ?? 0}` : call.toolCalls ? `— · ${call.toolCalls}` : "—"),
				},
				{ key: "msg", label: "消息", num: true, render: (call) => call.messages },
			];
			if (models.length > 1) columns.splice(3, 0, { key: "model", label: "模型", className: "sf-nowrap", render: (call) => call.model ?? "未选" });
			return h(
				Card,
				{
					title: "底层模型调用",
					sub: models.length === 1 && calls.length ? `管线各 Agent 经 DSH 调模型 · 全部经 ${models[0]}` : "管线各 Agent 经 DSH 调模型",
				},
				calls.length > 0 &&
					h(
						"div",
						{ className: "sf-stats" },
						stats.map(([label, value]) => h("span", { key: label }, `${label} `, h("b", null, value))),
						h("span", { className: "sf-muted" }, `（统计最近 ${calls.length} 次）`),
					),
				h(Table, { rows: shown, rowKey: (call) => call.seq, empty: "还没有调用。导演、角色、编剧、后处理的每次调用都会列在这里", columns }),
				toggle,
			);
		}

		function DebugDetails(props) {
			const { status, titles } = props;
			const [open, setOpen] = React.useState(false);
			const [calls, setCalls] = React.useState([]);
			const [events, setEvents] = React.useState([]);
			const load = React.useCallback(async () => {
				try {
					const [c, l] = await Promise.all([api("/debug/calls"), api("/debug/lifecycle")]);
					setCalls(c.calls ?? []);
					setEvents(l.events ?? []);
				} catch (e) {
					console.error("[storyforge] debug load failed", e);
				}
			}, []);
			React.useEffect(() => {
				if (open) void load();
			}, [open, load]);

			const bridge = status?.bridge;
			const active = status?.status?.active_campaign;
			const kv = (label, value) => [h("dt", { key: `${label}-k` }, label), h("dd", { key: `${label}-v` }, value ?? "—")];
			return h(
				"details",
				{ className: "sf-card sf-details", onToggle: (e) => setOpen(e.currentTarget.open) },
				h("summary", null, "调试信息", h("span", { className: "sf-card-sub" }, "写作引擎进程、适配器调用、会话生命周期")),
				open &&
					h(
						"div",
						{ className: "sf-debug" },
						h("div", { className: "sf-actions" }, h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => void load() }, "刷新调试数据")),
						h("div", { className: "sf-section-title" }, "写作引擎（sidecar）"),
						h(
							"dl",
							{ className: "sf-kv" },
							kv("状态", bridgeState(status).label),
							bridge?.pid && kv("进程", `pid ${bridge.pid}`),
							bridge?.lastExit && kv("上次退出", `code=${bridge.lastExit.code ?? "—"} signal=${bridge.lastExit.signal ?? "—"}`),
							kv("活跃存档", active ? `${titles[active] ?? ""} ${active}`.trim() : null),
							kv("工具协议", status?.status?.tool_mode ?? status?.toolMode),
							status?.status?.usage && kv("累计用量", describeUsage(status.status.usage)),
							bridge?.logFile && kv("日志", h("span", { className: "sf-mono" }, bridge.logFile)),
							status?.statusError && kv("错误", status.statusError),
						),
						h("div", { className: "sf-section-title" }, "适配器调用（DSH → StoryForge 适配器）"),
						h(Table, {
							rows: calls,
							rowKey: (call, i) => `${call.at}-${i}`,
							empty: "还没有调用",
							columns: [
								{ key: "at", label: "时间", className: "sf-nowrap", render: (call) => formatTime(call.at) },
								{ key: "s", label: "会话", render: (call) => h("span", { className: "sf-mono" }, call.sessionId ?? "—") },
								{ key: "p", label: "用途", render: (call) => call.purpose ?? "对话" },
								{ key: "m", label: "模型", render: (call) => `${call.provider}/${call.model}` },
								{ key: "n", label: "消息", num: true, render: (call) => call.messageCount },
								{ key: "t", label: "工具", num: true, render: (call) => call.toolNames?.length || "—" },
								{ key: "o", label: "开场", render: (call) => (call.opening ? "是" : "") },
								{ key: "i", label: "跳过的注入", render: (call) => (call.injected ?? []).join("、") },
							],
						}),
						h("div", { className: "sf-section-title" }, "StoryForge 会话生命周期"),
						events.length === 0
							? h("div", { className: "sf-muted" }, "暂无")
							: h(
									"ul",
									{ className: "sf-plain-list" },
									events.map((event, i) =>
										h(
											"li",
											{ key: i },
											h("span", { className: "sf-muted sf-nowrap" }, formatTime(event.at)),
											" ",
											h("span", { className: "sf-mono" }, event.sessionId),
											` source=${event.source}${event.parentSession ? ` 分叉自 ${event.parentSession}` : ""}`,
										),
									),
								),
					),
			);
		}

		/** 主区首页：新建故事、我的故事、管线设置、收尾与底层调用；调试信息折叠。 */
		function HomePanel() {
			const [models, setModels] = React.useState(null);
			const [cards, setCards] = React.useState(null);
			const [error, setError] = React.useState(null);
			const status = useBridgeStatus(true, 3000);
			const snapshot = useSessionList();
			const fake = status?.pipeline === "fake";

			const refresh = React.useCallback(async () => {
				try {
					const [m, k] = await Promise.all([api("/models"), api("/cards")]);
					setModels(m);
					setCards(k);
					setError(null);
				} catch (e) {
					setError(errorText(e));
				}
			}, []);
			React.useEffect(() => {
				void refresh();
			}, [refresh]);

			const stories = buildStories(status, snapshot);
			const titles = campaignTitles(stories);
			const state = bridgeState(status);
			return h(
				"div",
				{ className: "sf-root sf-home" },
				h(
					"div",
					{ className: "sf-home-inner" },
					h(
						"header",
						{ className: "sf-head" },
						h(
							"div",
							null,
							h("div", { className: "sf-title" }, "StoryForge"),
							h("div", { className: "sf-subtitle" }, "多 Agent 写作管线 · 模型调用全部经 DSH"),
						),
						h("div", { className: "sf-spacer" }),
						h(Badge, { tone: state.tone, title: state.title }, state.label),
						h(
							"button",
							{ type: "button", className: "sf-btn sf-btn-sm", title: "重新读取模型与角色卡列表", onClick: () => void refresh() },
							"刷新",
						),
					),
					error &&
						h(
							"div",
							{ className: "sf-error" },
							h("span", { className: "sf-grow" }, error),
							h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setError(null) }, "关闭"),
						),
					h(
						"div",
						{ className: "sf-grid" },
						h(NewStoryCard, {
							cards,
							fake,
							modelReady: Boolean(models?.selected),
							onCreated: () => void refresh(),
							onCardsChanged: () => api("/cards").then(setCards),
							onError: setError,
						}),
						!fake &&
							h(SettingsCard, {
								models,
								onError: setError,
								onChange: (patch) => setModels((current) => ({ ...current, ...patch })),
							}),
					),
					h(StoriesCard, { stories, status }),
					h(SettlesCard, { settles: status?.settles ?? [], titles }),
					h(LlmCallsCard, { calls: status?.llmCalls ?? [], total: status?.llmCallCount ?? 0 }),
					h(DebugDetails, { status, titles }),
				),
			);
		}
		//#endregion

		/** 左侧栏图标：一本打开的书。 */
		function PanelIcon(props) {
			const size = props?.size ?? 18;
			return h(
				"svg",
				{ width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.6, "aria-hidden": "true" },
				h("path", { d: "M12 6c-2-1.5-5-2-8-1.5v13c3-.5 6 0 8 1.5", strokeLinejoin: "round" }),
				h("path", { d: "M12 6c2-1.5 5-2 8-1.5v13c-3-.5-6 0-8 1.5V6z", strokeLinejoin: "round" }),
			);
		}

		/**
		 * 样式：颜色、圆角都取 DSH 主题变量（亮 / 暗主题各有一套值），自定义变量定义在
		 * 每个根元素上，跟随所在位置的主题；取不到时退回 currentColor 混色。
		 */
		const STYLES = `
.sf-root, .sf-btn, .sf-badge {
	--sf-fg: var(--dsw-alias-label-primary, inherit);
	--sf-fg2: var(--dsw-alias-label-secondary, inherit);
	--sf-fg3: var(--dsw-alias-label-tertiary, inherit);
	--sf-border: var(--dsw-alias-border-l2, color-mix(in srgb, currentColor 12%, transparent));
	--sf-border-strong: var(--dsw-alias-border-l3, color-mix(in srgb, currentColor 22%, transparent));
	--sf-card: var(--dsw-alias-settings-card-fill, transparent);
	--sf-hover: var(--dsw-alias-interactive-bg-hover, color-mix(in srgb, currentColor 6%, transparent));
	--sf-brand: var(--dsw-alias-brand-primary, #4d6bfe);
	--sf-on-brand: var(--dsw-alias-label-primary-foreground, #fff);
	--sf-ok: var(--dsw-alias-state-success-primary, #22c55e);
	--sf-warn: var(--dsw-alias-state-warn-primary, #f59e0b);
	--sf-err: var(--dsw-alias-state-error-primary, #ef4444);
	--sf-idle: var(--dsw-alias-label-tertiary, #8a8f98);
	--sf-radius: var(--dsw-radius-lg, 12px);
	--sf-radius-sm: var(--dsw-radius-sm, 6px);
}
.sf-root { color: var(--sf-fg); font-size: 13px; line-height: 1.6; }
.sf-home { height: 100%; box-sizing: border-box; overflow: auto; }
.sf-home-inner { max-width: 1120px; margin: 0 auto; padding: 20px 24px 40px; display: flex; flex-direction: column; gap: 16px; }
.sf-head { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.sf-title { font-size: 18px; font-weight: 600; line-height: 1.4; }
.sf-subtitle, .sf-card-sub { color: var(--sf-fg3); font-size: 12px; font-weight: 400; }
.sf-spacer, .sf-grow { flex: 1; }
.sf-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 16px; align-items: start; }
.sf-card { background: var(--sf-card); border: 1px solid var(--sf-border); border-radius: var(--sf-radius); padding: 16px 18px; min-width: 0; }
.sf-card-head { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; margin-bottom: 12px; }
.sf-card-title { font-size: 14px; font-weight: 600; }
.sf-card-actions { margin-left: auto; }
.sf-section-title { font-weight: 600; color: var(--sf-fg2); margin: 14px 0 6px; }
.sf-form { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 10px 12px; align-items: center; }
.sf-form > label { color: var(--sf-fg2); }
.sf-field { display: flex; align-items: center; gap: 8px; min-width: 0; }
.sf-hint { grid-column: 2; margin-top: -6px; color: var(--sf-fg3); font-size: 12px; }
.sf-input { height: 32px; box-sizing: border-box; width: 100%; max-width: 420px; min-width: 0; padding: 0 10px; border-radius: var(--sf-radius-sm); border: 1px solid var(--sf-border-strong); background: transparent; color: inherit; font: inherit; }
.sf-input:focus-visible, .sf-btn:focus-visible { outline: 2px solid color-mix(in srgb, var(--sf-brand) 50%, transparent); outline-offset: 1px; }
.sf-btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 32px; padding: 0 14px; box-sizing: border-box; border-radius: var(--sf-radius-sm); border: 1px solid var(--sf-border-strong); background: transparent; color: inherit; font: inherit; font-size: 13px; cursor: pointer; white-space: nowrap; }
.sf-btn:hover:not(:disabled) { background: var(--sf-hover); }
.sf-btn:disabled { opacity: 0.45; cursor: not-allowed; }
.sf-btn-primary { background: var(--sf-brand); border-color: var(--sf-brand); color: var(--sf-on-brand); font-weight: 500; }
.sf-btn-primary:hover:not(:disabled) { background: color-mix(in srgb, var(--sf-brand) 86%, #000); }
.sf-btn-sm { height: 28px; padding: 0 10px; font-size: 12px; }
.sf-actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-top: 14px; }
.sf-more { margin-top: 10px; }
.sf-gap-top { margin-top: 8px; }
.sf-badge { display: inline-flex; align-items: center; gap: 5px; padding: 0 8px; border-radius: 999px; font-size: 12px; line-height: 22px; white-space: nowrap; color: var(--sf-tone); background: color-mix(in srgb, var(--sf-tone) 14%, transparent); }
.sf-badge::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: currentColor; flex-shrink: 0; }
.sf-tone-ok { --sf-tone: var(--sf-ok); }
.sf-tone-warn { --sf-tone: var(--sf-warn); }
.sf-tone-err { --sf-tone: var(--sf-err); }
.sf-tone-run { --sf-tone: var(--sf-brand); }
.sf-tone-idle { --sf-tone: var(--sf-idle); }
.sf-tone-run::before { animation: sf-pulse 1.2s ease-in-out infinite; }
@keyframes sf-pulse { 50% { opacity: 0.25; } }
.sf-saved { color: var(--sf-ok); font-size: 12px; white-space: nowrap; }
.sf-error { display: flex; align-items: flex-start; gap: 10px; padding: 10px 12px; border-radius: var(--sf-radius-sm); background: color-mix(in srgb, var(--sf-err) 10%, transparent); color: var(--sf-err); white-space: pre-wrap; overflow-wrap: anywhere; }
.sf-error-line { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
.sf-row > .sf-input { flex: 1 1 auto; }
.sf-notice { display: flex; align-items: flex-start; gap: 10px; margin-top: 12px; padding: 10px 12px; border-radius: var(--sf-radius-sm); white-space: pre-wrap; overflow-wrap: anywhere; font-size: 13px; }
.sf-notice-ok { background: color-mix(in srgb, var(--sf-ok) 10%, transparent); }
.sf-notice-warn { background: color-mix(in srgb, var(--sf-warn) 14%, transparent); }
.sf-probe { margin-top: -4px; flex-wrap: wrap; }
.sf-probe-ok { color: var(--sf-ok); }
.sf-probe-err { color: var(--sf-err); overflow-wrap: anywhere; }
.sf-muted { color: var(--sf-fg3); }
.sf-small { font-size: 12px; }
.sf-nowrap { white-space: nowrap; }
.sf-mono { font-family: var(--dsw-font-family-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace); font-size: 12px; overflow-wrap: anywhere; }
.sf-empty { color: var(--sf-fg3); text-align: center; padding: 18px 0; }
.sf-table-wrap { overflow-x: auto; }
.sf-table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
.sf-table th { text-align: left; font-weight: 500; font-size: 12px; color: var(--sf-fg3); padding: 6px 8px; border-bottom: 1px solid var(--sf-border); white-space: nowrap; }
.sf-table td { padding: 7px 8px; border-bottom: 1px solid var(--sf-border); vertical-align: middle; }
.sf-table tbody tr:last-child td { border-bottom: 0; }
.sf-table tbody tr:hover td { background: var(--sf-hover); }
.sf-table .sf-num { text-align: right; white-space: nowrap; }
.sf-stats { display: flex; flex-wrap: wrap; gap: 4px 16px; margin-bottom: 8px; color: var(--sf-fg2); font-size: 12px; }
.sf-stats b { color: var(--sf-fg); font-weight: 600; }
.sf-stories { display: flex; flex-direction: column; }
.sf-story { display: flex; align-items: center; gap: 12px; padding: 10px 8px; border-radius: var(--sf-radius-sm); cursor: pointer; }
.sf-story:focus-visible { outline: 2px solid var(--sf-brand); outline-offset: -2px; }
.sf-story-missing { cursor: default; opacity: 0.7; }
.sf-chevron { color: var(--sf-fg3); font-size: 18px; line-height: 1; }
.sf-story + .sf-story { border-top: 1px solid var(--sf-border); }
.sf-story:not(.sf-story-missing):hover { background: var(--sf-hover); }
.sf-story-dim .sf-story-title { color: var(--sf-fg2); font-weight: 500; }
.sf-story-main { flex: 1; min-width: 0; }
.sf-story-title { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-story-meta { color: var(--sf-fg3); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-story-side { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
.sf-details > summary { cursor: pointer; list-style: none; display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 8px; font-size: 14px; font-weight: 600; }
.sf-details > summary::-webkit-details-marker { display: none; }
.sf-details > summary::before { content: "▸"; display: inline-block; color: var(--sf-fg3); transition: transform 0.15s; }
.sf-details[open] > summary::before { transform: rotate(90deg); }
.sf-kv { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 4px 12px; margin: 0; }
.sf-kv dt { color: var(--sf-fg3); }
.sf-kv dd { margin: 0; overflow-wrap: anywhere; }
.sf-plain-list { margin: 0; padding-left: 18px; }
.sf-panel { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
.sf-panel .sf-section-title { margin: 6px 0 0; }
.sf-panel-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.sf-settle-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.sf-settle-list li { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12px; }
.sf-view { padding: 20px 24px; display: flex; flex-direction: column; gap: 10px; max-width: 720px; font-size: 14px; line-height: 1.7; }
.sf-view p { margin: 0; }
.sf-dock { color: var(--sf-fg3); font-size: 12px; padding: 2px 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-mvu { display: flex; flex-direction: column; gap: 8px; margin-top: 6px; padding-top: 10px; border-top: 1px solid var(--sf-border); }
.sf-mvu .sf-section-title { margin: 0; }
.sf-mvu .sf-notice { margin-top: 0; }
.sf-mvu-section { display: flex; flex-direction: column; gap: 4px; }
.sf-mvu-section + .sf-mvu-section { margin-top: 6px; }
.sf-mvu-name { font-weight: 600; color: var(--sf-fg2); }
.sf-mvu-row { display: grid; grid-template-columns: minmax(56px, max-content) minmax(0, 1fr) max-content; align-items: center; gap: 8px; font-size: 12px; }
.sf-mvu-row > :nth-child(2):last-child { grid-column: 2 / 4; }
.sf-mvu-label { color: var(--sf-fg2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-mvu-bar { height: 8px; border-radius: 999px; background: color-mix(in srgb, var(--sf-tone) 16%, transparent); overflow: hidden; }
.sf-mvu-fill { display: block; height: 100%; border-radius: inherit; background: var(--sf-tone); transition: width 0.3s; }
.sf-mvu-value { color: var(--sf-fg3); font-variant-numeric: tabular-nums; }
.sf-mvu-text { overflow-wrap: anywhere; }
.sf-mvu-icon { font-size: 14px; }
.sf-mvu-tag { justify-self: start; padding: 0 8px; border-radius: 999px; line-height: 20px; background: var(--sf-hover); border: 1px solid var(--sf-border); }
.sf-mvu-actions { display: flex; flex-wrap: wrap; gap: 6px; }
.sf-mvu-fallbacks > summary { cursor: pointer; }
/* Turn 尾部的卡片 HTML：外层 contain: paint 是 fixed 定位的包含块，卡样式里的
   :host { position: fixed } 也逃不出这个框。 */
.sf-turn-tail { contain: paint; position: relative; max-width: 100%; overflow-x: auto; margin: 6px 0 2px; }
.sf-turn-text { white-space: pre-wrap; overflow-wrap: anywhere; color: var(--sf-fg2); font-size: 13px; }
.sf-turn-text > div + div { margin-top: 6px; }
.sf-turn-error { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.sf-shell { margin: 6px 0 2px; border: 1px solid var(--sf-border); border-radius: var(--sf-radius-sm); overflow: hidden; }
.sf-shell-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 4px 8px; border-bottom: 1px solid var(--sf-border); }
.sf-shell-head .sf-probe-err { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-shell-frame { display: block; width: 100%; border: 0; background: transparent; }
.sf-shell-note, .sf-shell-confirm { margin: 6px 0 2px; padding: 8px 10px; border-radius: var(--sf-radius-sm); display: flex; flex-direction: column; align-items: flex-start; gap: 6px; }
.sf-shell-note { border: 1px dashed var(--sf-border-strong); color: var(--sf-fg2); }
.sf-opening-text { display: grid; gap: 6px; }
.sf-opening-body { white-space: pre-wrap; line-height: 1.7; }
.sf-shell-confirm { background: color-mix(in srgb, var(--sf-warn) 12%, transparent); }
.sf-shell-proposals { display: flex; flex-direction: column; gap: 6px; padding: 6px 8px; border-top: 1px solid var(--sf-border); background: color-mix(in srgb, var(--sf-warn) 8%, transparent); }
.sf-shell-proposal { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
@media (max-width: 640px) {
	.sf-home-inner { padding: 16px 14px 32px; }
	.sf-grid { grid-template-columns: minmax(0, 1fr); }
	.sf-form { grid-template-columns: minmax(0, 1fr); }
	.sf-hint { grid-column: 1; }
	.sf-story-side { gap: 6px; }
	.sf-btn, .sf-input { height: 40px; }
}
`;

		// ─── Turn 尾部：卡片 HTML（状态栏）净化渲染 ────────────────────────────────
		//
		// 适配器在每条回复的 finish 块里带回放数据（这条回复在 StoryForge 存档里的位置 +
		// 从正文里拿掉的 HTML 块数），DSH 原样存进会话。下面的会话事件定义把它发布成 Turn 级
		// 数据；`conversation.chat.turnTail` 条目据此合批取展示数据，DOMPurify 净化后挂进
		// Shadow DOM（卡的样式关在里面）。净化不了时显示文字版，取数失败时给重试。

		const PROVIDER_ID = "storyforge";
		const TURN_DATA_KEY = "storyforge";
		const REPLAY_VERSION = 1;
		/** 一次 `/display` 最多带的节点数（服务端上限 200）。 */
		const DISPLAY_BATCH = 100;

		/** 会话事件定义：storyforge provider 的回复 → 这一 Turn 的回放数据。 */
		const storyforgeTurnDefinition = {
			// 发布数据的 key 必须等于 kind，否则会话事件流整条报错、对话区空白（阶段 0 实测）。
			kind: TURN_DATA_KEY,
			match: (event) => {
				if (event.type === "turn/start") return { id: String(event.data.turn), role: "start" };
				if (event.type === "assistant/message" && event.data?.message?.source?.provider === PROVIDER_ID)
					return { id: String(event.data.turn), role: "update" };
				return null;
			},
			start: (_context, match) => ({ turn: match.event.data.turn }),
			update: (context, match) => {
				const message = match.event.data.message;
				const ref = message.source?.replayState?.response?.storyforge;
				return {
					...(context.state ?? { turn: match.event.data.turn }),
					ref: ref?.v === REPLAY_VERSION && typeof ref.conversationId === "string" ? ref : null,
					messageId: message.id,
				};
			},
			// 回复落地之前（只有 turn/start）不发布，避免半截数据。
			buildLocationData: (context, scope, previous) => {
				if (scope !== "turn" || !context.state?.messageId) return null;
				if (previous?.kind === "turn" && previous.turn === context.state.turn && previous.value === context.state) return previous;
				return { kind: "turn", turn: context.state.turn, key: TURN_DATA_KEY, value: context.state };
			},
		};

		const noopSubscribe = () => () => {};

		function useTurnData(turn) {
			const source = turn?.data?.source?.(TURN_DATA_KEY);
			return React.useSyncExternalStore(
				source?.subscribe ?? noopSubscribe,
				source?.getSnapshot ?? (() => turn?.data?.get?.(TURN_DATA_KEY)),
			);
		}

		const refKey = (ref) => `${ref.conversationId}#${ref.nodeId ?? ""}#${ref.nodeIndex ?? ""}`;

		/**
		 * 一个会话的展示数据加载器：同一个宏任务里挂载的 Turn 尾部合成一次 `/display`。
		 * 展示正则的深度随对话变长而变——出现位置更靠后的节点（又写了一轮）时代数加一，
		 * 已挂载的尾部按新代数一起重取；重取期间先显示旧结果，不闪。
		 */
		function createDisplayStore(sessionId) {
			/** key → { ref, mounted, inflight, result?: { generation, item?, error? } } */
			const entries = new Map();
			const listeners = new Set();
			let generation = 0;
			let latestIndex = null;
			let timer = null;

			const notify = () => {
				for (const listener of listeners) listener();
			};
			const schedule = () => {
				timer ??= setTimeout(flush, 0);
			};
			const due = (entry) => entry.mounted > 0 && !entry.inflight && entry.result?.generation !== generation;

			function flush() {
				timer = null;
				const pending = [...entries].filter(([, entry]) => due(entry));
				for (let i = 0; i < pending.length; i += DISPLAY_BATCH) {
					const batch = pending.slice(i, i + DISPLAY_BATCH);
					const requested = generation;
					for (const [, entry] of batch) entry.inflight = true;
					postJson("/display", { sessionId, refs: batch.map(([key, entry]) => ({ ...entry.ref, key })) })
						.then(
							(body) => {
								for (const [key, entry] of batch) entry.result = { generation: requested, item: body.items?.[key] ?? null };
							},
							(error) => {
								for (const [, entry] of batch) entry.result = { generation: requested, error: errorText(error) };
							},
						)
						.finally(() => {
							for (const [, entry] of batch) entry.inflight = false;
							notify();
							if ([...entries.values()].some(due)) schedule();
						});
				}
			}

			return {
				subscribe(listener) {
					listeners.add(listener);
					return () => listeners.delete(listener);
				},
				result: (key) => entries.get(key)?.result,
				/** 已挂载的尾部里位置最靠后的节点（最新一轮）。 */
				latestIndex: () => latestIndex,
				register(key, ref) {
					let entry = entries.get(key);
					if (!entry) {
						entry = { ref, mounted: 0, inflight: false, result: undefined };
						entries.set(key, entry);
					}
					entry.mounted += 1;
					if (Number.isInteger(ref.nodeIndex)) {
						if (latestIndex !== null && ref.nodeIndex > latestIndex) generation += 1;
						if (latestIndex === null || ref.nodeIndex > latestIndex) latestIndex = ref.nodeIndex;
					}
					schedule();
					return () => {
						entry.mounted -= 1;
					};
				},
				retry(key) {
					const entry = entries.get(key);
					if (!entry) return;
					entry.result = undefined;
					schedule();
					notify();
				},
				/** 存档里的展示数据变了（开场界面改写了开场白）：已挂载的尾部按新代数重取。 */
				refresh() {
					generation += 1;
					schedule();
				},
			};
		}

		const displayStores = new Map();

		function displayStore(sessionId) {
			let store = displayStores.get(sessionId);
			if (!store) {
				store = createDisplayStore(sessionId);
				displayStores.set(sessionId, store);
			}
			return store;
		}

		/** 与桌面 RichContent 同一份净化规则；返回 DOM 片段直接挂载，不再二次解析字符串。 */
		const SANITIZE_CONFIG = Object.freeze({
			USE_PROFILES: { html: true },
			ALLOW_ARIA_ATTR: true,
			ALLOW_DATA_ATTR: true,
			FORBID_TAGS: ["script", "iframe", "object", "embed", "link", "meta", "base", "form", "input", "button", "textarea", "select", "option"],
			FORBID_ATTR: [
				"onabort", "onblur", "onchange", "onclick", "ondblclick", "onerror", "onfocus", "oninput", "onkeydown", "onkeypress",
				"onkeyup", "onload", "onmousedown", "onmouseenter", "onmouseleave", "onmousemove", "onmouseout", "onmouseover", "onmouseup", "onsubmit",
			],
			RETURN_DOM_FRAGMENT: true,
		});

		let purifyLoading = null;

		/** 按需加载插件自带的 DOMPurify（同源路由，ES 模块）。失败后下次再试。 */
		function loadPurify() {
			if (!purifyLoading) {
				purifyLoading = import(`${API_BASE}/vendor/purify.js`).then((module) => {
					const purify = module.default;
					if (!purify?.isSupported) throw new Error("DOMPurify 在这个浏览器里不可用");
					// 卡里的链接在新标签页打开，不把 DSH 页面导航走。
					purify.addHook("afterSanitizeAttributes", (node) => {
						if (node.tagName === "A" && node.hasAttribute("href")) {
							node.setAttribute("target", "_blank");
							node.setAttribute("rel", "noopener noreferrer");
						}
					});
					return purify;
				});
				purifyLoading.catch(() => {
					purifyLoading = null;
				});
			}
			return purifyLoading;
		}

		/** Shadow DOM 里的底样式：字色字号跟随 DSH，卡的样式只作用在这棵树里。 */
		const CARD_BASE_STYLES = `
:host { display: block; }
.sf-block + .sf-block { margin-top: 8px; }
.sf-block * { max-width: 100%; box-sizing: border-box; }
.sf-block img { height: auto; }
.sf-block table { border-collapse: collapse; }
`;

		/** 一组净化后的卡片 HTML 块。内容不变时不重挂（保住用户展开的 `<details>`）。 */
		function CardBlocks({ blocks }) {
			const hostRef = React.useRef(null);
			const [failed, setFailed] = React.useState(false);
			const content = JSON.stringify(blocks.map((block) => block.html));
			React.useLayoutEffect(() => {
				const host = hostRef.current;
				if (!host) return undefined;
				let cancelled = false;
				loadPurify()
					.then((purify) => {
						if (cancelled) return;
						const root = host.shadowRoot ?? host.attachShadow({ mode: "open" });
						const style = document.createElement("style");
						style.textContent = CARD_BASE_STYLES;
						const rendered = blocks.map((block) => {
							const wrapper = document.createElement("div");
							wrapper.className = "sf-block";
							wrapper.append(purify.sanitize(block.html, SANITIZE_CONFIG));
							return wrapper;
						});
						if (!rendered.some((wrapper) => wrapper.textContent.trim() || wrapper.querySelector("img, svg, progress, meter"))) {
							setFailed(true);
							return;
						}
						root.replaceChildren(style, ...rendered);
					})
					.catch((error) => {
						console.error("[storyforge] card html render failed", error);
						if (!cancelled) setFailed(true);
					});
				return () => {
					cancelled = true;
				};
				// 按内容比较：每次重取 blocks 都是新数组。
			}, [content]);
			if (failed) {
				return h(
					"div",
					{ className: "sf-root sf-turn-tail sf-turn-text" },
					blocks.map((block, index) => h("div", { key: index }, block.text)),
				);
			}
			return h("div", { className: "sf-turn-tail", "data-sf-turn-tail": "" }, h("div", { ref: hostRef }));
		}

		// ─── Turn 尾部：卡片界面（档 3）──────────────────────────────────────────────
		//
		// 卡的界面（`$('body').load(url)` 状态栏、带脚本的内联 HTML）跑在 sandbox iframe 里，只给
		// `allow-scripts`，永远不给 `allow-same-origin`。文档由服务端拼好、登记成一次性路径（响应头
		// CSP 带 sandbox）；这里挂 iframe、应答壳的桥请求：代取远程资源（服务端按白名单）、读
		// Campaign 变量树（只读）、存壳自己的界面状态（写回存档）、写变量（先成提案，用户点
		// 「应用」才写，桌面 M4 同口径）、读世界书条目与切换启用状态、开场界面选定开场。
		// SillyTavern / TavernHelper API 请求交给桌面的 `createHostHandler`，需要后端的方法一律
		// 报不支持（桌面上卡壳虚拟插件同样调不动后端）。

		const SHELL_OPENING_KINDS = new Set(["opening_home", "opening_custom"]);
		const SHELL_DEFAULT_HEIGHT = { status: 120, message_html: 280 };
		const SHELL_MIN_HEIGHT = 60;
		/** 内容按视口高度排版的界面会把报上来的高度越撑越大，到这里为止。 */
		const SHELL_MAX_HEIGHT = 1200;
		const MAX_SHELL_PROPOSALS = 50;
		/** 桌面卡壳虚拟插件的权限（`CardShellHost.vue`）。 */
		const SHELL_PLUGIN_PERMISSIONS = ["ReadVariables", "WriteVariables", "ReadMemory", "ReadCharacter", "ModifyPrompt", "Generate"];
		const SHELL_UNSUPPORTED = {
			opening_chat_applied: "只有开局前的开场界面能选择开场",
		};

		let cardShellLoading = null;

		/** 按需加载桌面卡片界面模块的打包（同源路由，ES 模块）。失败后下次再试。 */
		function loadCardShellModule() {
			if (!cardShellLoading) {
				cardShellLoading = import(`${API_BASE}/vendor/card-shell.js`);
				cardShellLoading.catch(() => {
					cardShellLoading = null;
				});
			}
			return cardShellLoading;
		}

		/** 消息来自这个壳或壳里嵌套的子 iframe：沿 parent 链找壳窗口（桌面 L3 同口径，不信消息里的字段）。 */
		function fromShellWindow(source, shellWindow) {
			if (!source || !shellWindow) return false;
			try {
				let w = source;
				for (let i = 0; w && i < 10; i++) {
					if (w === shellWindow) return true;
					if (w === w.parent) break;
					w = w.parent;
				}
			} catch (_) {}
			return false;
		}

		function textToDataUrl(text, contentType) {
			const bytes = new TextEncoder().encode(text);
			let binary = "";
			for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
			return `data:${contentType || "text/plain"};base64,${btoa(binary)}`;
		}

		function previewShellValue(value, max = 60) {
			let text;
			if (value === undefined || value === null) text = "null";
			else if (typeof value === "string") text = value;
			else {
				try {
					text = JSON.stringify(value);
				} catch {
					text = String(value);
				}
			}
			return text.length > max ? `${text.slice(0, max - 1)}…` : text;
		}

		/** 同一个键只留最新一条提案；壳刷不同的键时丢最旧的（桌面 `upsertShellVariableProposal`）。 */
		function upsertProposal(list, payload) {
			const key = String(payload?.key || "").trim();
			if (!key) return list;
			const next = list.filter((proposal) => proposal.key !== key);
			next.push({ id: `${key}:${Date.now()}:${Math.random().toString(36).slice(2, 6)}`, key, value: payload?.value });
			return next.length > MAX_SHELL_PROPOSALS ? next.slice(next.length - MAX_SHELL_PROPOSALS) : next;
		}

		/**
		 * 壳的桥请求 → 宿主动作。抛错会作为 error 回给壳。`opened` 是服务端挂载时的回执（开场
		 * 界面带 `opening` 与桥会话），`onOpeningApplied` 在开场白改写后刷新尾部。
		 */
		async function answerShellRequest(sessionId, type, payload, { opened, onOpeningApplied } = {}) {
			switch (type) {
				case "fetch_text": {
					const { resource } = await postJson("/shell/fetch", { sessionId, url: payload.url });
					if (resource.body_text == null) throw new Error(`not text: ${payload.url}`);
					return resource.body_text;
				}
				case "fetch_data_url": {
					const { resource } = await postJson("/shell/fetch", { sessionId, url: payload.url });
					const contentType = resource.content_type || "application/octet-stream";
					if (resource.body_base64 != null) return `data:${contentType};base64,${resource.body_base64}`;
					if (resource.body_text != null) return textToDataUrl(resource.body_text, contentType);
					throw new Error("empty");
				}
				case "mvu_status":
					return { ready: true, runtime: "StoryForge DSH（只读）" };
				case "mvu_data_get":
					return (await postJson("/shell/mvu-data", { sessionId })).data;
				case "shell_variables_set":
					await postJson("/shell/variables", { sessionId, selector: payload.selector, variables: payload.variables, mode: payload.mode });
					return { ok: true };
				case "campaign_worldbook_get":
					return (await postJson("/shell/worldbook", { sessionId, name: payload.name })).entries;
				case "campaign_worldbook_update": {
					const { updated } = await postJson("/shell/worldbook-update", { sessionId, name: payload.name, entries: payload.entries });
					return { updated };
				}
				case "opening_chat_applied": {
					if (!opened?.opening) break;
					await postJson("/shell/opening", { sessionId, bridgeSession: opened.bridgeSession, swipe_id: payload.swipe_id, mes: payload.mes });
					onOpeningApplied?.();
					return { ok: true };
				}
				default:
					throw new Error(SHELL_UNSUPPORTED[type] ?? `unknown bridge type: ${type}`);
			}
			throw new Error(SHELL_UNSUPPORTED[type] ?? `unknown bridge type: ${type}`);
		}

		/** 一个挂好的卡片界面：iframe + 状态行 + 变量提案确认条。开场界面按视口定高（桌面同口径）。 */
		function ShellFrame({ sessionId, turnRef, part, approved, onOpeningApplied }) {
			const frameRef = React.useRef(null);
			const [attempt, setAttempt] = React.useState(0);
			const [opened, setOpened] = React.useState(null);
			const [error, setError] = React.useState(null);
			const [runtimeError, setRuntimeError] = React.useState(null);
			const [height, setHeight] = React.useState(SHELL_DEFAULT_HEIGHT[part.kind] ?? SHELL_DEFAULT_HEIGHT.message_html);
			const [proposals, setProposals] = React.useState([]);
			const [applying, setApplying] = React.useState(null);
			const [notice, setNotice] = React.useState(null);
			const shell = part.mode === "load" ? { mode: "load", url: part.url } : { mode: "inline", start: part.start };
			const identity = JSON.stringify([sessionId, turnRef, shell, approved === true, attempt]);

			React.useEffect(() => {
				let alive = true;
				setOpened(null);
				setError(null);
				setRuntimeError(null);
				Promise.all([loadCardShellModule(), postJson("/shell/open", { sessionId, ref: turnRef, shell, approved: approved === true })]).then(
					([module, body]) => alive && setOpened({ ...body, module }),
					(e) => alive && setError(errorText(e)),
				);
				return () => {
					alive = false;
				};
			}, [identity]);

			React.useEffect(() => {
				if (!opened) return undefined;
				const { module } = opened;
				const shellWindow = () => frameRef.current?.contentWindow;
				const stHandler = module.createHostHandler(
					{ id: opened.pluginId, permissions: SHELL_PLUGIN_PERMISSIONS },
					async (command) => {
						throw new Error(`DSH 的卡片界面不能调用 ${command}`);
					},
					{ isTrustedSource: (event) => fromShellWindow(event?.source, shellWindow()) },
				);
				const onMessage = (event) => {
					if (!fromShellWindow(event.source, shellWindow())) return;
					const data = event.data;
					if (data?.type === module.MSG_REQUEST && data.pluginId === opened.pluginId) {
						void stHandler(event);
						return;
					}
					if (!module.isCardShellBridgeMessageForSession(event, opened.bridgeSession)) return;
					const payload = data.payload || {};
					if (data.type === "shell_resize") {
						if (opened.opening) return;
						const next = Math.min(SHELL_MAX_HEIGHT, Math.max(SHELL_MIN_HEIGHT, Math.ceil(Number(payload.height) || 0)));
						setHeight((current) => (Math.abs(current - next) > 2 ? next : current));
						return;
					}
					if (data.type === "var_write") {
						setProposals((list) => upsertProposal(list, payload));
						return;
					}
					if (data.type === "shell_runtime") {
						if (payload.kind === "module_error" || payload.kind === "preload_error") setRuntimeError(String(payload.detail || payload.kind));
						return;
					}
					if (!data.id) return;
					const reply = (result, failure) => {
						try {
							event.source.postMessage({ __sf_shell_bridge_res: data.id, result, error: failure || null }, "*");
						} catch (_) {}
					};
					answerShellRequest(sessionId, data.type, payload, { opened, onOpeningApplied }).then(
						(result) => reply(result, null),
						(e) => reply(null, errorText(e)),
					);
				};
				window.addEventListener("message", onMessage);
				return () => window.removeEventListener("message", onMessage);
			}, [opened, sessionId, onOpeningApplied]);

			const applyProposal = (proposal) => {
				setApplying(proposal.id);
				setNotice(null);
				postJson("/shell/var-write", { sessionId, key: proposal.key, value: proposal.value })
					.then(
						({ result }) => {
							if (result.ok) {
								setProposals((list) => list.filter((item) => item.id !== proposal.id));
								setNotice(`已写入「${result.key}」`);
							} else setNotice(`「${proposal.key}」没写进去（提案保留）：${result.error}`);
						},
						(e) => setNotice(`「${proposal.key}」没写进去（提案保留）：${errorText(e)}`),
					)
					.finally(() => setApplying(null));
			};

			const label = opened?.label ?? part.label;
			const frameHeight = opened?.opening
				? Number.parseInt(opened.module.getOpeningShellPresentation(window.innerHeight).height, 10)
				: height;
			return h(
				"div",
				{ className: "sf-root sf-shell", "data-sf-shell": part.kind },
				h(
					"div",
					{ className: "sf-shell-head sf-small sf-muted" },
					h("span", null, label),
					!opened && !error ? h("span", null, "加载中…") : null,
					error ? h("span", { className: "sf-probe-err", title: error }, `打不开：${error}`) : null,
					runtimeError ? h("span", { className: "sf-probe-err", title: runtimeError }, `界面脚本出错：${runtimeError}`) : null,
					h("span", { className: "sf-spacer" }),
					h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setAttempt((n) => n + 1) }, "重新加载"),
				),
				opened
					? h("iframe", {
							ref: frameRef,
							className: "sf-shell-frame",
							src: `${API_BASE}/shell/doc?t=${encodeURIComponent(opened.token)}`,
							sandbox: "allow-scripts",
							referrerPolicy: "no-referrer",
							loading: "lazy",
							title: label,
							style: { height: frameHeight },
						})
					: null,
				proposals.length > 0
					? h(
							"div",
							{ className: "sf-shell-proposals sf-small" },
							h("div", null, "卡片界面请求修改变量（点「应用」才会写进存档）："),
							proposals.map((proposal) =>
								h(
									"div",
									{ key: proposal.id, className: "sf-shell-proposal" },
									h("span", { className: "sf-mono" }, `${proposal.key} → ${previewShellValue(proposal.value)}`),
									h(
										"button",
										{ type: "button", className: "sf-btn sf-btn-sm", disabled: applying !== null, onClick: () => applyProposal(proposal) },
										applying === proposal.id ? "写入中…" : "应用",
									),
									h(
										"button",
										{
											type: "button",
											className: "sf-btn sf-btn-sm",
											disabled: applying !== null,
											onClick: () => setProposals((list) => list.filter((item) => item.id !== proposal.id)),
										},
										"忽略",
									),
								),
							),
						)
					: null,
				notice ? h("div", { className: "sf-small sf-muted", style: { padding: "4px 8px" } }, notice) : null,
			);
		}

		/**
		 * 一条回复里的一个卡片界面：卡清单里的开场界面只在开局前挂（之后只留说明）；不在卡
		 * 清单里的先确认；较早轮次的状态栏界面默认收起（每轮都有一份，全挂太重），最新一轮的
		 * 直接挂。
		 */
		function ShellPart({ sessionId, turnRef, part, latest, onOpeningApplied }) {
			const [approved, setApproved] = React.useState(false);
			const [expanded, setExpanded] = React.useState(false);
			if (SHELL_OPENING_KINDS.has(part.kind) && part.opening === "closed") {
				return h(
					"div",
					{ className: "sf-root sf-shell-note sf-small", "data-sf-shell": part.kind },
					`这条回复带着卡的${part.label}（开场设置）。故事已经开始，开场界面只在写第一轮之前可用。`,
				);
			}
			if (!part.trusted && !approved) {
				return h(
					"div",
					{ className: "sf-root sf-shell-confirm sf-small", "data-sf-shell-confirm": "" },
					part.mode === "load"
						? h(React.Fragment, null, h("div", null, "这条回复要挂一个不在本卡清单里的界面页面："), h("code", { className: "sf-mono" }, part.url))
						: h("div", null, "这条回复带着可执行的内联界面，但它没对上本卡的正则触发器（可能是模型自己写出来的）。"),
					h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setApproved(true) }, "信任并挂载本次"),
				);
			}
			if (part.kind === "status" && !latest && !expanded) {
				return h(
					"div",
					{ className: "sf-root sf-shell-note sf-small" },
					h("div", null, `${part.label}（较早的一轮，默认收起）`),
					h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => setExpanded(true) }, "展开"),
				);
			}
			return h(ShellFrame, { sessionId, turnRef, part, approved, onOpeningApplied });
		}

		/** 开场界面改写了开场白：DSH 正文里还是开局时的那份，新开场的文字补在尾部。 */
		function OpeningText({ text }) {
			return h(
				"div",
				{ className: "sf-root sf-turn-tail sf-opening-text" },
				h("div", { className: "sf-small sf-muted" }, "开场（按开场界面的选择）"),
				h("div", { className: "sf-opening-body" }, text),
			);
		}

		/** `conversation.chat.turnTail` 条目：StoryForge 会话里有卡片 HTML 块或卡片界面的 Turn 才渲染。 */
		function StoryForgeTurnTail(props) {
			const sessionId = props.sfSessionId ?? props.sessionId;
			const isStoryForge = useIsStoryForge(props, sessionId);
			const data = useTurnData(props.turn);
			const ref = isStoryForge && sessionId && (data?.ref?.blocks > 0 || data?.ref?.shells > 0) ? data.ref : null;
			const key = ref ? refKey(ref) : null;
			const store = key ? displayStore(sessionId) : null;
			React.useEffect(() => (store ? store.register(key, ref) : undefined), [store, key]);
			const result = React.useSyncExternalStore(store?.subscribe ?? noopSubscribe, () => store?.result(key));
			if (!store || !result) return null;
			if (result.error) {
				return h(
					"div",
					{ className: "sf-root sf-turn-tail sf-small sf-muted sf-turn-error" },
					h("span", null, `状态栏没取到：${result.error}`),
					h("button", { type: "button", className: "sf-btn sf-btn-sm", onClick: () => store.retry(key) }, "重试"),
				);
			}
			const parts = result.item?.parts ?? [];
			if (parts.length === 0) return null;
			const latest = Number.isInteger(ref.nodeIndex) && ref.nodeIndex === store.latestIndex();
			return h(
				React.Fragment,
				null,
				parts.map((part, index) =>
					part.type === "text"
						? h(OpeningText, { key: `text:${index}`, text: part.text })
						: part.type === "html"
							? h(CardBlocks, { key: `html:${index}`, blocks: part.blocks })
							: h(ShellPart, {
									key: part.mode === "load" ? `load:${part.url}` : `inline:${part.start}`,
									sessionId,
									turnRef: ref,
									part,
									latest,
									onOpeningApplied: store.refresh,
								}),
				),
			);
		}

		const inject = ["slots", "sessions", "sidebarRight", "sidebarRightTabs", "uiWorkspace", "uiConversation"];

		function apply(ctx) {
			rootCtx = ctx;

			ctx.effect(() => {
				const style = document.createElement("style");
				style.dataset.storyforge = "";
				style.textContent = STYLES;
				document.head.appendChild(style);
				return () => style.remove();
			}, "storyforge: styles");

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

			// Turn 尾部：卡片 HTML。会话事件定义随插件生命周期登记、卸载时自动撤销。
			ctx.uiConversation.events.register(storyforgeTurnDefinition);
			ctx.slots.inject("conversation.chat.turnTail", () =>
				ctx.slots.register(
					{
						name: "conversation.chat.turnTail",
						id: "storyforge-card-html",
						inject: (sessionId) => ({ sfSessionId: sessionId }),
					},
					StoryForgeTurnTail,
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
