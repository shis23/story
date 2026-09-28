import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

function readUtf8(filePath) {
  return fs.readFileSync(filePath, 'utf8')
}

function listRustSources(root) {
  const sources = []
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) visit(entryPath)
      else if (entry.isFile() && entry.name.endsWith('.rs')) sources.push(entryPath)
    }
  }
  visit(root)
  return sources.sort()
}

function listFrontendSources(root) {
  const sources = []
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) visit(entryPath)
      else if (entry.isFile() && /\.(?:js|mjs|vue)$/.test(entry.name)) sources.push(entryPath)
    }
  }
  visit(root)
  return sources.sort()
}

export function extractRegisteredCommands(source) {
  const match = source.match(/tauri::generate_handler!\[([\s\S]*?)\]\s*\)\s*\.run/)
  if (!match) throw new Error('tauri::generate_handler! registration block not found')

  const commands = []
  for (const line of match[1].split(/\r?\n/)) {
    const entryName = line.replace(/\/\/.*$/, '').trim().replace(/,$/, '').trim()
    if (/^(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*$/.test(entryName)) {
      commands.push(entryName.split('::').at(-1))
    }
  }
  return commands
}

/// 只匹配"整行就是一个命令属性"的行——注释或字符串里提到
/// `#[tauri::command]` 不算命令。
///
/// 2026-09-13 域4：`extractCommandAttributes` 原先用无锚点的全局正则，于是
/// **文档注释里出现 `#[tauri::command]` 字样就会被计成一个命令**。实测：在
/// `export_campaign_st_cards` 的注释里写明"非 async 的 `#[tauri::command]`…"
/// 之后，commandAttributes 立刻从 175 变成 176、并让新加的
/// "定义数 == 注册数" 门禁误报（`export_campaign_st_cards_impl` 被当成未注册
/// 命令）。凡"数量类事实"的门禁都必须按属性行统计，否则注释就能让计数漂移。
const COMMAND_ATTRIBUTE_LINE = /^\s*#\[tauri::command(?:\([^\]]*\))?\]/

export function extractCommandAttributes(source) {
  return source
    .split(/\r?\n/)
    .filter((line) => COMMAND_ATTRIBUTE_LINE.test(line)).length
}

export function extractFrontendInvokes(source) {
  const found = []
  // Direct invoke('cmd', ...) in JS/Vue script blocks.
  for (const m of source.matchAll(/\binvoke\(\s*['"]([^'"]+)['"]/g)) found.push(m[1])
  // shellDoc / adapter wrappers call `_invoke('cmd', ...)` in two shapes:
  //   this._invoke('cmd', ...)                       (member call)
  //   const token = await _invoke('cmd', ...)        (module-local binding)
  // frontend/src/utils/shellDocUrl.js uses the second shape, so the old
  // `\._invoke\(` pattern (which required a literal dot) silently skipped it.
  // `(?<![\w])` accepts both while still rejecting identifiers that merely end
  // in `_invoke`. NOTE: `\binvoke\(` above cannot match `_invoke(` because the
  // `_`/`i` pair has no word boundary, so the two patterns never double count.
  //
  // 2026-09-13 域4 修复 T-01：旧写法漏掉 shellDocUrl.js 的三次裸 `_invoke(...)`
  // 调用（card_shell_register_doc / card_shell_register_module /
  // card_shell_unregister_doc），使 uniqueInvokeCount 停在假值 169，
  // 并让 tauri-command-contract.test.mjs 的
  // `missingBackendCommands == []` 断言对这 3 个 live 调用点永久失明。
  // 修正后真实值 172。
  for (const m of source.matchAll(/(?<![\w])_invoke\(\s*['"]([^'"]+)['"]/g)) found.push(m[1])
  // Dynamic command tables (plugin-bridge.js API_METHODS) declare command: '...'.
  // (?<![\w]) keeps `command:` from matching inside `slash_command:`.
  for (const m of source.matchAll(/(?<![\w])command:\s*['"]([^'"]+)['"]/g)) found.push(m[1])
  // Only well-formed command identifiers count; dynamic concatenations like
  // `_invoke(' + commandName + ', ...)` are not statically resolvable.
  return found.filter((name) => /^[a-z_][a-z0-9_]*$/.test(name))
}

/**
 * Commands that are deliberately allowed to live outside `commands/*.rs`.
 *
 * 2026-09-13 域4 修复 T-13/T-16：docs/DOCS-CODE-AUDIT.md 声称"全部 175 个命令
 * 位于 crates/tauri-app/src/commands/*.rs"，实测 156 个在 commands/、19 个在
 * crates/tauri-app/src/card_studio_api.rs。位置本身可以接受，但在本门禁之前
 * **没有任何断言保护它**——Gate 1 只断言 lib.rs 内没有 `#[tauri::command]`
 * （frontend/tests/tauri-command-contract.test.mjs），所以这 19 个命令可以被
 * 静默移动到任何地方，或者在改动中被整体删除而文档仍写着 commands/。
 * 这里把它们显式登记：任何新增的"commands/ 之外"的命令都会让门禁失败，
 * 必须在此处登记 + 说明理由（并同步 docs 的位置描述）。
 */
export const COMMAND_LOCATION_ALLOWLIST = {
  'card_studio_api.rs': [
    // Card Studio Phase 1（19 个）。
    'cardstudio_list_projects',
    'cardstudio_create_project',
    'cardstudio_create_from_novel',
    'cardstudio_create_from_character',
    'cardstudio_prefill_from_novel',
    'cardstudio_get_project',
    'cardstudio_delete_project',
    'cardstudio_update_artifacts',
    'cardstudio_set_stage',
    'cardstudio_set_options',
    'cardstudio_run_checks',
    'cardstudio_run_review',
    'cardstudio_compile',
    'cardstudio_export_gate',
    'cardstudio_export_png',
    'cardstudio_complete_manual_stage',
    'cardstudio_run_stage',
    'cardstudio_import_compiled',
    'cardstudio_list_stages',
  ],
}

/**
 * 后端已注册、但当前没有任何前端入口的命令——**显式声明的保留 API**。
 *
 * 2026-09-13 域4 修复 T-15：这 3 个命令的 wrapper 已于 2026-09-01 删除，但
 * 命令与 `generate_handler!` 注册条目被留下，此前属于"静默遗留"——没有任何
 * 文档或断言记录它们是有意保留还是漏删。这里把它们变成被声明的状态：
 * 任何**不在本表**里的零入口命令都会让 evaluateGates 失败（见
 * `undeclaredOrphanCommands`），因此新增孤儿必须显式登记并说明理由。
 *
 * 保留（而非删除）的决策依据见
 * docs/review-2026-09-13/fixes/04-tauri-fixes.md：abandon_turn 承载 Gate 8
 * 复评加固的 CAS 谓词、archive_conversation 删除会连带死掉
 * archive_conversation_impl、且删除需要跨 3 个不可写文件同步（frontend 测试
 * ×1 + docs ×2）。若将来要删，需要连删：命令 fn + `lib.rs` 注册条目 +
 * `frontend/tests/fixtures/tauri-registered-commands.snapshot.json` 条目
 * （`npm`-free 路径：`node scripts/architecture/backend-baseline.mjs
 * --write-snapshot`）+ 本表条目 + 文档中的命令计数（README / DOCS-CODE-AUDIT）。
 */
export const RETAINED_NO_FRONTEND_CALLER = [
  {
    command: 'abandon_turn',
    file: 'commands/turns.rs',
    reason: 'Gate 8 复评的 Turn CAS 谓词（防 abandon 回退已 Committed 的正文变体）',
  },
  {
    command: 'archive_conversation',
    file: 'commands/conversations.rs',
    reason: '唯一调用点为自身 wrapper；删除需连带删除 archive_conversation_impl',
  },
  {
    command: 'soft_delete_variant',
    file: 'commands/turns.rs',
    reason: 'Discard Attempt 的手动入口；store 方法仍被 backend_workflows 补偿路径使用',
  },
  // ─── 2026-09-13 域5 死代码清理后新增的零入口命令（20 条）────────────────────
  //
  // 背景：域5（review-frontend）按 Lead 裁决 A「协同删除」移除了 21 个孤儿前端
  // wrapper（20 删 + cardstudioListStages 接线保留）。删除后这 20 个后端命令从
  // 「有 wrapper 但无人调用」变为「零前端入口」，触发本文件的门禁
  // `undeclaredOrphanCommands` → 因此在**这里声明**。
  //
  // 重要边界：本表声明的是**事实**（后端注册 + 前端无入口），不是对「应当保留」的
  // 背书。是否彻底删除这 20 个命令需 Lead 裁定；若裁定删除，按本表上方注释的连删清单
  // 执行（命令 fn + `lib.rs` 注册条目 + `--write-snapshot` + 本表条目 + 文档命令计数）。
  // 反过来若某命令重新接线，删掉本表条目即可——`staleRetainedDeclarations` 会立刻提示。
  {
    command: 'add_world_info_entry',
    file: 'commands/characters.rs',
    reason: '角色卡世界书条目新增（Campaign 维度另有 add_campaign_world_info_entry）',
  },
  {
    command: 'update_world_info_entry',
    file: 'commands/characters.rs',
    reason: '角色卡世界书条目更新（Campaign 维度另有 update_campaign_world_info_entry）',
  },
  {
    command: 'delete_world_info_entry',
    file: 'commands/characters.rs',
    reason: '角色卡世界书条目删除（Campaign 维度另有 delete_campaign_world_info_entry）',
  },
  {
    command: 'update_world_info_route',
    file: 'commands/characters.rs',
    reason: '世界书触发路由更新（Campaign 维度另有 set_campaign_world_info_route）',
  },
  {
    command: 'card_shell_allow_host',
    file: 'commands/card_shell.rs',
    reason: 'T-09 提权白名单入口：保留，重新接线时必须走用户确认（不可静默放行）',
  },
  {
    command: 'configure_embedder',
    file: 'commands/connections.rs',
    reason: '嵌入模型配置写入；能力已实现、UI 未接（域5 独立核对：非"被取代"，属零引用）',
  },
  {
    command: 'get_embed_config',
    file: 'commands/connections.rs',
    reason: '嵌入模型配置读取；能力已实现、UI 未接（域5 独立核对：非"被取代"，属零引用）',
  },
  {
    command: 'delete_character',
    file: 'commands/characters.rs',
    reason:
      'CLAUDE.md 文档化的级联删除语义（对 StoredCharacter.id / source_character_id / 同会话 tool_ctx 域 id 做 Campaign/MVU/向量清理）——属文档化保留 API，不是随手遗留（域5 建议保留）',
  },
  {
    command: 'export_st_card_png',
    file: 'commands/import_export.rs',
    reason: '单卡导出 ST PNG（当前 UI 走 Campaign 级批量导出 export_campaign_st_cards）',
  },
  {
    command: 'get_active_agent_profile_config',
    file: 'commands/profiles.rs',
    reason: 'Agent Profile 当前配置读取',
  },
  {
    command: 'get_active_profile',
    file: 'commands/profiles.rs',
    reason: 'Profile 当前项读取',
  },
  {
    command: 'get_active_preset',
    file: 'commands/presets.rs',
    reason: 'Preset 当前项读取',
  },
  {
    command: 'list_modules',
    file: 'commands/profiles.rs',
    reason: '模块列表读取',
  },
  {
    command: 'list_profiles',
    file: 'commands/profiles.rs',
    reason: 'Profile 列表读取',
  },
  {
    command: 'save_profile',
    file: 'commands/profiles.rs',
    reason: 'Profile 保存',
  },
  {
    command: 'set_active_profile',
    file: 'commands/profiles.rs',
    reason: 'Profile 切换',
  },
  {
    command: 'update_module',
    file: 'commands/profiles.rs',
    reason: '模块更新',
  },
  {
    command: 'log_get_llm_call',
    file: 'commands/diagnostics.rs',
    reason: 'LLM 调用日志单条读取（诊断面板走 list/clear 路径）',
  },
  {
    command: 'meta_classify_st_preset',
    file: 'commands/meta_typed.rs',
    reason: 'ST preset 分类（Meta 分析辅助命令）',
  },
  {
    command: 'meta_get_conversation',
    file: 'commands/meta.rs',
    reason: 'Meta 会话读取',
  },
]

/**
 * Map every `#[tauri::command]` function name to its workspace-relative file.
 *
 * `sources` entries are `{ relativePath, source }` and must already exclude test
 * modules, so a `#[tauri::command]` that only exists behind `#[cfg(test)]`
 * cannot masquerade as a production command definition.
 */
export function extractCommandLocations(sources) {
  const locations = {}
  for (const { relativePath, source } of sources) {
    const lines = source.split(/\r?\n/)
    for (let index = 0; index < lines.length; index += 1) {
      if (!COMMAND_ATTRIBUTE_LINE.test(lines[index])) continue
      for (let probe = index + 1; probe < Math.min(index + 26, lines.length); probe += 1) {
        const match = lines[probe].match(/\bfn\s+([A-Za-z0-9_]+)\s*\(/)
        if (match) {
          locations[match[1]] = relativePath
          break
        }
      }
    }
  }
  return locations
}

/** Commands defined outside `commands/*.rs` and not in the allowlist above. */
export function findCommandsOutsideAllowedLocations(commandLocations) {
  const violations = []
  for (const [name, file] of Object.entries(commandLocations)) {
    if (file.startsWith('commands/')) continue
    const allowed = COMMAND_LOCATION_ALLOWLIST[file] ?? []
    if (!allowed.includes(name)) violations.push({ name, file })
  }
  return violations.sort((a, b) => a.name.localeCompare(b.name))
}

/**
 * Gate evaluation for the CLI entry point, so
 * `node scripts/architecture/backend-baseline.mjs` is itself a runnable check
 * and not merely a snapshot printer.
 */
export function evaluateGates(baseline) {
  const failures = []
  const missing = baseline.frontend.missingBackendCommands
  if (missing.length > 0) {
    // 前端调用但后端未注册必须为 0。T-01 修正扫描后这一条才真正覆盖
    // shellDocUrl.js 的 3 个裸 `_invoke` 调用点。
    failures.push(`前端 invoke 但后端未注册 (${missing.length}): ${missing.join(', ')}`)
  }
  const duplicates = baseline.backend.duplicateRegisteredCommands
  if (duplicates.length > 0) {
    failures.push(`generate_handler! 重复注册 (${duplicates.length}): ${duplicates.join(', ')}`)
  }
  const defined = baseline.backend.definedCommandCount
  if (baseline.backend.registeredCommandCount !== defined) {
    failures.push(
      `定义数 (${defined}) != 注册数 (${baseline.backend.registeredCommandCount})`,
    )
  }
  const outside = baseline.backend.commandsOutsideAllowedLocations
  if (outside.length > 0) {
    failures.push(
      '命令位于 commands/ 之外且未登记白名单: ' +
        outside.map((v) => `${v.name} (${v.file})`).join(', '),
    )
  }
  const undeclared = baseline.backend.undeclaredOrphanCommands
  if (undeclared.length > 0) {
    // 零入口命令必须是"被声明的保留 API"（T-15）。新增孤儿必须登记进
    // RETAINED_NO_FRONTEND_CALLER，否则门禁失败——静默遗留到此为止。
    failures.push(
      `零前端入口但未登记为保留 API (${undeclared.length}): ${undeclared.join(', ')}`,
    )
  }
  // N-R3-01：反向一致性也必须是门禁的一部分。原先只算不算（仅在契约测试里
  // 断言），单独跑脚本时"保留声明已过期"（命令已重新接线或已被删除）不会让
  // exit code 变红 ⇒ 门禁存在静默漏检。这里补齐。
  const stale = baseline.backend.staleRetainedDeclarations
  if (stale.length > 0) {
    failures.push(
      `保留声明已过期（命令已重新接线或已删除，应删除对应登记）(${stale.length}): ${stale.join(', ')}`,
    )
  }
  return failures
}

export function extractSqliteReferences(source) {
  const lines = source.split(/\r?\n/)
  const unsupported = []
  lines.forEach((line, index) => {
    if (
      /ensure_json_meta_backend_supported|ensure_typed_patch_backend_supported|sqlite backend skips|unsupported until an atomic SQLite Meta UoW/i.test(
        line,
      )
    ) {
      unsupported.push({ line: index + 1, text: line.trim() })
    }
  })
  return {
    activeFlagReferences: (source.match(/is_sqlite_active\(/g) ?? []).length,
    unsupported,
  }
}

export function collectBaseline(repoRoot = REPO_ROOT) {
  const libPath = path.join(repoRoot, 'crates', 'tauri-app', 'src', 'lib.rs')
  const backendSourceRoot = path.join(repoRoot, 'crates', 'tauri-app', 'src')
  const apiPath = path.join(repoRoot, 'frontend', 'src', 'tauri-api.js')
  const cargoPath = path.join(repoRoot, 'Cargo.toml')
  const libSource = readUtf8(libPath)
  const backendSources = listRustSources(backendSourceRoot).map((filePath) => ({
    filePath,
    source: readUtf8(filePath),
  }))
  const backendSource = backendSources.map(({ source }) => source).join('\n')
  const productionBackendSources = backendSources.filter(
    ({ filePath }) => !path.basename(filePath).startsWith('lib_tests'),
  )
  const productionBackendSource = productionBackendSources
    .map(({ source }) => source)
    .join('\n')
  const apiSource = readUtf8(apiPath)
  const cargoSource = readUtf8(cargoPath)
  const registered = extractRegisteredCommands(libSource)
  // Gate 8 复评：前端 invoke 扫描覆盖全部 frontend/src（tauri-api.js 静态
  // invoke、shellDoc/adapter 的 ._invoke、plugin-bridge.js 的 command: 动态
  // 表、.vue 直调），不再只看单一文件。
  const frontendSourceRoot = path.join(repoRoot, 'frontend', 'src')
  const frontend = listFrontendSources(frontendSourceRoot).flatMap((filePath) =>
    extractFrontendInvokes(readUtf8(filePath)),
  )
  const registeredSet = new Set(registered)
  const frontendSet = new Set(frontend)
  const sqlite = extractSqliteReferences(productionBackendSource)
  sqlite.facadeFlagReferences = productionBackendSources
    .filter(({ filePath }) => ['sqlite_runtime.rs', 'storage_backend.rs'].includes(path.basename(filePath)))
    .reduce(
      (count, { source }) => count + (source.match(/is_sqlite_active\(/g) ?? []).length,
      0,
    )
  sqlite.applicationFlagReferences =
    sqlite.activeFlagReferences - sqlite.facadeFlagReferences
  sqlite.ambientCharacterStoreReferences = productionBackendSources
    .filter(({ filePath }) => {
      const relative = path.relative(backendSourceRoot, filePath)
      return relative.startsWith(`commands${path.sep}`) || path.basename(filePath) === 'runtime_support.rs'
    })
    .reduce((count, { source }) => count + (source.match(/\bget_store\(\)/g) ?? []).length, 0)
  const selectedWriterConstructorPattern =
    /\b(?:CampaignStore|CharacterStore|TurnStore|CompressJobStore)::new\(/g
  sqlite.facadeSelectedWriterConstructors = productionBackendSources
    .filter(({ filePath }) => path.basename(filePath) === 'storage_backend.rs')
    .reduce(
      (count, { source }) => count + (source.match(selectedWriterConstructorPattern) ?? []).length,
      0,
    )
  sqlite.applicationSelectedWriterConstructors = productionBackendSources
    .filter(({ filePath }) => {
      const relative = path.relative(backendSourceRoot, filePath)
      return relative.startsWith(`commands${path.sep}`) || path.basename(filePath) === 'runtime_support.rs'
    })
    .reduce(
      (count, { source }) => count + (source.match(selectedWriterConstructorPattern) ?? []).length,
      0,
    )
  // Gate 3: `.is_sqlite()` / `.is_json()` method calls are only allowed in the
  // bootstrap file, the facade, the SQLite runtime and the named backend
  // adapter. Every other production source must be zero.
  const backendFlagPattern = /\.is_sqlite\(\)|\.is_json\(\)/g
  const methodFlagFiles = new Map()
  for (const { filePath, source } of productionBackendSources) {
    const count = (source.match(backendFlagPattern) ?? []).length
    if (count > 0) methodFlagFiles.set(path.relative(backendSourceRoot, filePath).replaceAll('\\', '/'), count)
  }
  const methodFlagWhitelist = [
    'lib.rs',
    'storage_backend.rs',
    'sqlite_runtime.rs',
    'backend_workflows.rs',
    // 三审9：启动恢复协调模块（按 is_sqlite 分派 + JSON 路径 facade stores），
    // 与 backend_workflows 同性质——原 lib.rs:243 包装器抽取而来。
    'startup_recovery.rs',
  ]
  sqlite.applicationMethodFlagReferences = [...methodFlagFiles.entries()]
    .filter(([name]) => !methodFlagWhitelist.includes(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.facadeMethodFlagReferences = [...methodFlagFiles.entries()]
    .filter(([name]) => methodFlagWhitelist.includes(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.methodFlagReferencesByFile = Object.fromEntries(methodFlagFiles)
  // Direct legacy JSON store accessors (json_character_store / json_campaign_store /
  // json_turn_store / json_compress_job_store) must be confined to the facade +
  // named backend adapter (methodFlagWhitelist). commands/* is NO LONGER a
  // whitelist: the Gate-5 review-followup replaced every command-side
  // `.ok()`-hidden json_character_store access with the backend-neutral
  // `collect_scoped_regex_scripts_for_backend` resolver (stored/source/card/name
  // mapping through the facade). Any reference in commands, card_studio_api,
  // playthrough_lifecycle, runtime_support, … is a backend-policy leak: it makes
  // the path JSON-only and silently breaks under SQLite authority.
  const legacyStoreAccessorPattern =
    /\.json_(?:character|campaign|turn|compress_job)_store\b/g
  const legacyAccessorFiles = new Map()
  for (const { filePath, source } of productionBackendSources) {
    const count = (source.match(legacyStoreAccessorPattern) ?? []).length
    if (count > 0)
      legacyAccessorFiles.set(
        path.relative(backendSourceRoot, filePath).replaceAll('\\', '/'),
        count,
      )
  }
  const legacyAccessorAllowed = (name) => methodFlagWhitelist.includes(name)
  sqlite.applicationLegacyStoreAccessorReferences = [...legacyAccessorFiles.entries()]
    .filter(([name]) => !legacyAccessorAllowed(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.legacyStoreAccessorReferencesByFile = Object.fromEntries(legacyAccessorFiles)
  const crates = [...cargoSource.matchAll(/^\s*"(crates\/[^"\r\n]+)"\s*,?\s*$/gm)].map(
    (match) => match[1],
  )

  // 2026-09-13 域4 T-13/T-16：命令位置基线（详见 COMMAND_LOCATION_ALLOWLIST）。
  const commandLocations = extractCommandLocations(
    productionBackendSources.map(({ filePath, source }) => ({
      relativePath: path.relative(backendSourceRoot, filePath).replaceAll('\\', '/'),
      source,
    })),
  )
  const commandsOutsideAllowedLocations =
    findCommandsOutsideAllowedLocations(commandLocations)
  // 后端已注册但前端零入口 = 孤儿命令。其中被 RETAINED_NO_FRONTEND_CALLER
  // 显式声明的属于"有意保留的 API"（T-15）；未声明的会触发门禁失败。
  // tauri-api.js 之外的入口（plugin-bridge 动态表、shellDocUrl 的 _invoke、
  // MvuJsRuntime.vue 直调）都已被 extractFrontendInvokes 覆盖，所以这里的
  // 结果就是"真孤儿"。（2026-09-13 域4 时点：3 个，全部已声明。）
  const orphanRegisteredCommands = [...registeredSet]
    .filter((name) => !frontendSet.has(name))
    .sort()
  const retainedSet = new Set(RETAINED_NO_FRONTEND_CALLER.map((entry) => entry.command))
  const undeclaredOrphanCommands = orphanRegisteredCommands.filter(
    (name) => !retainedSet.has(name),
  )
  // 反向一致性：声明为"保留"但实际已重新接上入口（或已被删除）的命令，
  // 属于声明过期，也要暴露出来。
  const staleRetainedDeclarations = [...retainedSet]
    .filter((name) => !registeredSet.has(name) || frontendSet.has(name))
    .sort()

  return {
    generatedAt: new Date().toISOString(),
    files: {
      backend: 'crates/tauri-app/src/**/*.rs',
      frontendApi: 'frontend/src/tauri-api.js',
    },
    backend: {
      libLines: libSource.split(/\r?\n/).length,
      commandAttributes: extractCommandAttributes(backendSource),
      definedCommandCount: Object.keys(commandLocations).length,
      registeredCommandCount: registeredSet.size,
      duplicateRegisteredCommands: [...new Set(registered.filter((name, index) => registered.indexOf(name) !== index))].sort(),
      registeredCommands: [...registeredSet].sort(),
      commandLocations,
      commandsOutsideAllowedLocations,
      retainedNoFrontendCaller: RETAINED_NO_FRONTEND_CALLER.map((entry) => entry.command).sort(),
      undeclaredOrphanCommands,
      staleRetainedDeclarations,
    },
    frontend: {
      invokeCount: frontend.length,
      uniqueInvokeCount: frontendSet.size,
      invokedCommands: [...frontendSet].sort(),
      missingBackendCommands: [...frontendSet].filter((name) => !registeredSet.has(name)).sort(),
    },
    orphanRegisteredCommands,
    workspace: { crateCount: crates.length, crates },
    sqlite,
  }
}

if (process.argv[1]?.endsWith('backend-baseline.mjs')) {
  const baseline = collectBaseline()
  process.stdout.write(`${JSON.stringify(baseline, null, 2)}\n`)

  const failures = evaluateGates(baseline)

  // 快照再生成入口（T-10）：命令表变化时不再靠手工誊抄。
  //   node scripts/architecture/backend-baseline.mjs --write-snapshot
  //
  // CI 安全：**默认运行（无参数）永不写文件**——只有显式带 `--write-snapshot`
  // 才会写，且这里额外要求门禁全通过。原因：本脚本会先打印 JSON、再求值门禁；
  // 若在"定义≠注册/存在未声明孤儿/命令越界"的违规状态下仍允许写快照，就会把
  // 违规状态固化成 `frontend/tests/fixtures/tauri-registered-commands.snapshot.json`，
  // 让"快照深比较"这条测试失去意义。因此违规时**拒绝写入**并提示先修门禁。
  if (process.argv.includes('--write-snapshot')) {
    const snapshotPath = path.join(
      REPO_ROOT,
      'frontend',
      'tests',
      'fixtures',
      'tauri-registered-commands.snapshot.json',
    )
    if (failures.length > 0) {
      process.stderr.write(
        '[backend-baseline] --write-snapshot 已跳过：门禁未通过，拒绝把违规状态写进快照\n',
      )
    } else {
      const snapshot = [...baseline.backend.registeredCommands]
      fs.writeFileSync(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
      process.stderr.write(
        `[backend-baseline] 已写入快照 ${snapshot.length} 条：${path.relative(REPO_ROOT, snapshotPath)}\n`,
      )
    }
  }

  process.stderr.write(
    `[backend-baseline] 定义 ${baseline.backend.definedCommandCount} / 注册 ` +
      `${baseline.backend.registeredCommandCount} / 前端唯一 invoke ` +
      `${baseline.frontend.uniqueInvokeCount} / 孤儿命令 ` +
      `${baseline.orphanRegisteredCommands.length}` +
      (baseline.orphanRegisteredCommands.length
        ? ` (${baseline.orphanRegisteredCommands.join(', ')})`
        : '') +
      '\n',
  )
  if (failures.length > 0) {
    process.stderr.write(
      `[backend-baseline] 门禁失败 ${failures.length} 项:\n` +
        failures.map((line) => `  - ${line}`).join('\n') +
        '\n',
    )
    process.exitCode = 1
  } else {
    process.stderr.write('[backend-baseline] 门禁通过\n')
  }
}
