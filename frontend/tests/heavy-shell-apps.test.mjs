import test from 'node:test'
import assert from 'node:assert/strict'
import {
  HEAVY_TH_INLINE_BYTES,
  heavyShellSizeLabel,
  isHeavyThShell,
  splitHeavyThShells,
} from '../src/utils/heavyShellApps.js'

function thInline(label, byteLen, extra = {}) {
  return {
    kind: 'tavern_helper_module',
    label,
    entry: { inline_js: { js: '', deferred: true, byte_len: byteLen } },
    ...extra,
  }
}

test('isHeavyThShell keys on inline_js byte volume', () => {
  // 已知重型应用（57-99K）判重；逻辑脚本（<30K）判轻
  assert.equal(isHeavyThShell(thInline('bgm 播放器', 57_000)), true)
  assert.equal(isHeavyThShell(thInline('图鉴', 99_000)), true)
  assert.equal(isHeavyThShell(thInline('MagVarUpdate', 18_000)), false)
  assert.equal(isHeavyThShell(thInline('边界', HEAVY_TH_INLINE_BYTES)), true)
  assert.equal(isHeavyThShell(thInline('边界下', HEAVY_TH_INLINE_BYTES - 1)), false)

  // byte_len 缺失时回退 js.length
  const withJs = {
    kind: 'tavern_helper_module',
    label: 'inline-full',
    entry: { inline_js: { js: 'x'.repeat(40_000) } },
  }
  assert.equal(isHeavyThShell(withJs), true)

  // 非 TH / 非 inline_js / 空值一律不算重型
  assert.equal(
    isHeavyThShell({ kind: 'status_bar', entry: { remote_url: { url: 'https://x' } } }),
    false,
  )
  assert.equal(
    isHeavyThShell({ kind: 'tavern_helper_module', entry: { remote_url: { url: 'https://x' } } }),
    false,
  )
  assert.equal(isHeavyThShell(null), false)
})

test('splitHeavyThShells partitions without reordering or copying', () => {
  const logic = thInline('逻辑', 10_000)
  const bgm = thInline('bgm', 60_000)
  const status = { kind: 'status_bar', label: '状态', entry: { remote_url: { url: 'https://x' } } }
  const { light, heavy } = splitHeavyThShells([logic, bgm, status])
  assert.deepEqual(light, [logic, status])
  assert.deepEqual(heavy, [bgm])
  // 原对象引用（TavernHelperRuntime 依赖 raw shell 结构）
  assert.equal(heavy[0], bgm)

  assert.deepEqual(splitHeavyThShells(null), { light: [], heavy: [] })
})

test('heavyShellSizeLabel renders kilobytes', () => {
  assert.equal(heavyShellSizeLabel(thInline('a', 57_344)), '56K')
  assert.equal(heavyShellSizeLabel({ kind: 'tavern_helper_module', entry: {} }), '')
})
