// 卡片界面（档 3）要用到的桌面前端模块：`scripts/build-vendor.mjs` 用 esbuild 把它们打成
// `lib/vendor/card-shell.mjs`（入库；`test/vendor.test.mjs` 重新打包比对，桌面源码一改就要
// 重新生成）。插件运行时不读 `frontend/`，只读打好的这一份。
//
// 服务端用它切分消息里的卡片界面、判断信任、拼壳文档、解析开场界面的选择与世界书切换；
// 浏览器半边用其中的 `createHostHandler` 应答壳里的 SillyTavern / TavernHelper API 请求，
// 用 `getOpeningShellPresentation` 给开场界面定高。

export {
  generateBridgeScript,
  createHostHandler,
  MSG_REQUEST,
  MSG_RESPONSE,
} from '../../../frontend/src/plugin-bridge.js'
export {
  rewriteCardShellTopBridgeAccess,
  createCardShellRuntimeCompatibilityScript,
  isCardShellBridgeMessageForSession,
} from '../../../frontend/src/utils/cardShellDocument.js'
export {
  createCardShellFetchProxyScript,
  createCardShellMapReadyFallbackScript,
} from '../../../frontend/src/utils/cardShellFetchProxy.js'
export {
  segmentShellContent,
  trustedInlineShellDocStarts,
  classifyShellUrl,
} from '../../../frontend/src/utils/cardShellDisplay.js'
export {
  CARD_SHELL_VARIABLES_KEY,
  normalizeCardShellVariables,
  updateCardShellVariables,
  mergeCardShellVariables,
  patchCardShellVariables,
} from '../../../frontend/src/utils/cardShellVariableStore.js'
export { buildMvuStatDataTree } from '../../../frontend/src/utils/mvuStatTree.js'
export { analyzeEsmModuleSource } from '../../../frontend/src/utils/tavernHelperScripts.js'
export {
  buildOpeningChatSeed,
  resolveOpeningChatSelection,
} from '../../../frontend/src/utils/cardShellOpeningChat.js'
export { buildGreetingOptionsFromDetail } from '../../../frontend/src/utils/campaignGreetingOptions.js'
export {
  mapCampaignWorldbookForTavernHelper,
  resolveCampaignWorldbookEnabledUpdates,
} from '../../../frontend/src/utils/cardShellWorldbook.js'
export { getOpeningShellPresentation } from '../../../frontend/src/utils/cardShellPresentation.js'
