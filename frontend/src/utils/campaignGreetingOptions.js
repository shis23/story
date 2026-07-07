export function buildGreetingOptionsFromDetail(detail) {
  if (!detail) return []

  const options = []
  const seen = new Set()
  const addOption = (label, content) => {
    if (!content || !content.trim() || seen.has(content)) return
    seen.add(content)
    options.push({ label, content })
  }

  addOption('默认', detail.first_mes)
  const alternates = Array.isArray(detail.alternate_greetings) ? detail.alternate_greetings : []
  alternates.forEach((content, index) => {
    addOption(`备选 ${index + 1}`, content)
  })
  return options
}
