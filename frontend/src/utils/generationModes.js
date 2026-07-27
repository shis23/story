export const generationModeCatalog = Object.freeze([
  Object.freeze({
    value: 'continuation',
    label: '续写',
    hint: '单笔者',
    callEstimate: '1 次正文 + 2 次廉价记账',
  }),
  Object.freeze({
    value: 'duet',
    label: '对手戏',
    hint: 'A-B-A',
    callEstimate: '3–4 次正文编排 + 2 次廉价记账',
  }),
  Object.freeze({
    value: 'sequential_crew',
    label: '顺序剧组',
    hint: '逐角接戏',
    callEstimate: '2+N 次顺序编排 + 2 次廉价记账',
    featured: true,
  }),
])

export function generationModeCostLabel(value) {
  return generationModeCatalog.find((mode) => mode.value === value)?.callEstimate || ''
}
