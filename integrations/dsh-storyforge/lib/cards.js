/**
 * 角色卡文件：从 PNG / JSON 里读出卡名（首页下拉框显示卡名而不是文件名），以及导入时
 * 的文件名清洗与内容校验。只读不执行：PNG 只解析 tEXt 块里的 `ccv3` / `chara`。
 */

import { extname } from 'node:path'

/** 首页一次导入的单卡上限（PNG 卡常带大图，几 MB 是常态）。 */
export const MAX_CARD_BYTES = 20 * 1024 * 1024

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

/** 卡 JSON（V1 顶层 name / V2、V3 的 data.name）→ 卡名；不是卡时返回 null。 */
function nameOfCardJson(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const name = value.data?.name ?? value.name
  return typeof name === 'string' && name.trim() ? name.trim() : null
}

const stripBom = (text) => (text.charCodeAt(0) === 0xfeff ? text.slice(1) : text)

function parseJson(text) {
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

/** PNG 的 tEXt 块：keyword → 文本（latin1）。遇到 IEND 或结构损坏即停。 */
function pngTextChunks(bytes) {
  const chunks = new Map()
  if (bytes.length < PNG_SIGNATURE.length || !bytes.subarray(0, 8).equals(PNG_SIGNATURE)) return null
  let offset = 8
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset)
    const type = bytes.toString('latin1', offset + 4, offset + 8)
    const start = offset + 8
    const end = start + length
    if (end + 4 > bytes.length) break
    if (type === 'tEXt') {
      const data = bytes.subarray(start, end)
      const nul = data.indexOf(0)
      if (nul > 0) chunks.set(data.toString('latin1', 0, nul), data.toString('latin1', nul + 1))
    }
    if (type === 'IEND') break
    offset = end + 4
  }
  return chunks
}

/**
 * 卡文件内容 → 卡名。PNG 先读 V3（`ccv3`）再读 V2（`chara`），都是 base64 编码的 JSON。
 * @returns 卡名；不是可识别的角色卡时返回 null。
 */
export function readCardName(bytes, ext) {
  const kind = String(ext).toLowerCase()
  if (kind === '.json') return nameOfCardJson(parseJson(stripBom(bytes.toString('utf8'))))
  if (kind !== '.png') return null
  const chunks = pngTextChunks(bytes)
  if (!chunks) return null
  for (const keyword of ['ccv3', 'chara']) {
    const encoded = chunks.get(keyword)
    if (!encoded) continue
    const name = nameOfCardJson(parseJson(Buffer.from(encoded, 'base64').toString('utf8')))
    if (name) return name
  }
  return null
}

/**
 * 用户给的文件名 → 卡目录里安全的文件名：只留最后一段，去掉控制字符与 Windows 保留
 * 字符，限制长度；扩展名必须是 .png / .json。
 * @returns 清洗后的文件名；扩展名不对时返回 null。
 */
export function sanitizeCardFileName(raw) {
  const base = String(raw ?? '')
    .split(/[\\/]/)
    .pop()
    .normalize('NFC')
  const ext = extname(base).toLowerCase()
  if (ext !== '.png' && ext !== '.json') return null
  let stem = base
    .slice(0, base.length - ext.length)
    .replace(/[\u0000-\u001f\u007f<>:"|?*]/g, '')
    .replace(/^[\s.]+|[\s.]+$/g, '')
  if (/^(con|prn|aux|nul|com\d|lpt\d)$/i.test(stem)) stem = `${stem}_`
  stem = Array.from(stem).slice(0, 80).join('') || 'card'
  return `${stem}${ext}`
}

/** 目录里已有同名文件时依次试 `名字 (2).png`、`名字 (3).png`…… */
export function nextFreeName(file, exists) {
  if (!exists(file)) return file
  const ext = extname(file)
  const stem = file.slice(0, file.length - ext.length)
  for (let n = 2; n < 1000; n++) {
    const candidate = `${stem} (${n})${ext}`
    if (!exists(candidate)) return candidate
  }
  throw new Error(`卡目录里同名文件太多：${file}`)
}
