function defaultDocumentRef() {
  return typeof document === 'undefined' ? null : document
}

export function createDocumentKeydownController({
  documentRef = defaultDocumentRef,
  key = 'Escape',
  onKey,
  capture = true,
} = {}) {
  let activeDocument = null
  let activeHandler = null

  function disable() {
    if (!activeDocument || !activeHandler) return
    activeDocument.removeEventListener('keydown', activeHandler, capture)
    activeDocument = null
    activeHandler = null
  }

  function enable() {
    disable()
    const doc = documentRef?.()
    if (!doc?.addEventListener || !doc?.removeEventListener) return

    activeDocument = doc
    activeHandler = (event) => {
      if (event.key === key) onKey?.(event)
    }
    activeDocument.addEventListener('keydown', activeHandler, capture)
  }

  return {
    enable,
    disable,
    dispose: disable,
    isEnabled: () => Boolean(activeHandler),
  }
}
