// 测试用：手工拼一张带 tEXt 块的 PNG 角色卡。

import { deflateSync } from 'node:zlib'

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  return c >>> 0
})

function crc32(bytes) {
  let c = 0xffffffff
  for (const byte of bytes) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const head = Buffer.alloc(8)
  head.writeUInt32BE(data.length, 0)
  head.write(type, 4, 'latin1')
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, 'latin1'), data])), 0)
  return Buffer.concat([head, data, crc])
}

/** 1×1 PNG，附带若干 tEXt 块（keyword → JSON 对象，base64 编码，与 SillyTavern 一致）。 */
export function pngCard(texts) {
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(1, 0)
  ihdr.writeUInt32BE(1, 4)
  ihdr[8] = 8
  ihdr[9] = 2
  const pixels = deflateSync(Buffer.from([0, 0, 0, 0]))
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    ...Object.entries(texts).map(([keyword, value]) =>
      chunk('tEXt', Buffer.concat([Buffer.from(`${keyword}\0`, 'latin1'), Buffer.from(Buffer.from(JSON.stringify(value)).toString('base64'), 'latin1')])),
    ),
    chunk('IDAT', pixels),
    chunk('IEND', Buffer.alloc(0)),
  ])
}
