/**
 * dsh-plugin-storyforge — 浏览器半边（PoC）
 *
 * P0：左侧栏「StoryForge」入口 + 主区调试面板（宿主连通性、最近的适配器调用）。
 * P1 起在此加入：哨兵 + 动态注册的 StoryForge 视图、右栏标签、停靠区状态条。
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

		async function getJson(path) {
			const res = await fetch(`${API_BASE}${path}`, { credentials: "same-origin" });
			if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
			return res.json();
		}

		const styles = {
			root: {
				height: "100%",
				boxSizing: "border-box",
				overflow: "auto",
				padding: "16px 20px",
				fontSize: 13,
			},
			head: { display: "flex", alignItems: "center", gap: 12, marginBottom: 12 },
			title: { fontSize: 15, fontWeight: 600 },
			muted: { opacity: 0.6, fontSize: 12 },
			button: {
				all: "unset",
				cursor: "pointer",
				padding: "3px 10px",
				borderRadius: 6,
				fontSize: 12,
				border: "1px solid color-mix(in srgb, currentColor 25%, transparent)",
			},
			table: { width: "100%", borderCollapse: "collapse", fontVariantNumeric: "tabular-nums" },
			cell: {
				textAlign: "left",
				padding: "4px 8px",
				borderBottom: "1px solid color-mix(in srgb, currentColor 12%, transparent)",
				verticalAlign: "top",
			},
		};

		/** 主区调试面板：宿主连通性 + 最近的适配器调用。 */
		function DebugPanel() {
			const [ping, setPing] = React.useState(null);
			const [calls, setCalls] = React.useState([]);
			const [error, setError] = React.useState(null);

			const refresh = React.useCallback(async () => {
				try {
					const [p, c] = await Promise.all([getJson("/ping"), getJson("/debug/calls")]);
					setPing(p);
					setCalls(c.calls ?? []);
					setError(null);
				} catch (e) {
					setError(String(e?.message ?? e));
				}
			}, []);

			React.useEffect(() => {
				void refresh();
			}, [refresh]);

			const header = ["时间", "会话", "用途", "模型", "消息数", "工具"];
			return h("div", { style: styles.root }, [
				h("div", { key: "head", style: styles.head }, [
					h("span", { key: "t", style: styles.title }, "StoryForge PoC 调试"),
					h(
						"span",
						{ key: "s", style: styles.muted },
						ping ? `宿主在线 · provider=${ping.provider} model=${ping.model} pid=${ping.pid}` : "连接中…",
					),
					h("button", { key: "r", type: "button", style: styles.button, onClick: () => void refresh() }, "刷新"),
				]),
				error && h("div", { key: "err", style: { color: "#ff8a80", marginBottom: 8 } }, error),
				h("table", { key: "tbl", style: styles.table }, [
					h(
						"thead",
						{ key: "h" },
						h("tr", null, header.map((label) => h("th", { key: label, style: styles.cell }, label))),
					),
					h(
						"tbody",
						{ key: "b" },
						calls.length === 0
							? h("tr", null, h("td", { colSpan: header.length, style: styles.cell }, "还没有适配器调用"))
							: calls.map((call, i) =>
									h("tr", { key: i }, [
										h("td", { key: "at", style: styles.cell }, call.at.slice(11, 19)),
										h("td", { key: "s", style: styles.cell }, call.sessionId ?? "—"),
										h("td", { key: "p", style: styles.cell }, call.purpose ?? "对话"),
										h("td", { key: "m", style: styles.cell }, `${call.provider}/${call.model}`),
										h("td", { key: "n", style: styles.cell }, String(call.messageCount)),
										h(
											"td",
											{ key: "tools", style: styles.cell },
											call.toolNames.length === 0 ? "无" : call.toolNames.join(", "),
										),
									]),
								),
					),
				]),
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

		const inject = ["slots"];

		function apply(ctx) {
			ctx.slots.inject("sidebar.panellist", () =>
				ctx.slots.register(
					{ name: "sidebar.panellist", id: "storyforge", order: 13, label: "StoryForge" },
					PanelIcon,
				),
			);
			ctx.slots.inject("main", () => ctx.slots.register({ name: "main", key: "storyforge" }, DebugPanel));
		}

		exports.apply = apply;
		exports.inject = inject;
		return module.exports;
	},
});
