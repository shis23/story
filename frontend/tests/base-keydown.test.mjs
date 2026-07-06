import test from 'node:test'
import assert from 'node:assert/strict'
import { createDocumentKeydownController } from '../src/components/base/documentKeydownController.js'

function createDocumentStub() {
  const listeners = []
  return {
    listeners,
    addEventListener(type, handler, capture) {
      listeners.push({ type, handler, capture, active: true })
    },
    removeEventListener(type, handler, capture) {
      const listener = listeners.find(
        (entry) => entry.active
          && entry.type === type
          && entry.handler === handler
          && entry.capture === capture,
      )
      if (listener) listener.active = false
    },
    dispatchKey(key) {
      for (const listener of listeners.filter((entry) => entry.active && entry.type === 'keydown')) {
        listener.handler({ key })
      }
    },
    activeCount() {
      return listeners.filter((entry) => entry.active).length
    },
  }
}

test('document keydown controller does not accumulate listeners across enable calls', () => {
  const doc = createDocumentStub()
  let calls = 0
  const controller = createDocumentKeydownController({
    documentRef: () => doc,
    key: 'Escape',
    onKey: () => { calls += 1 },
  })

  controller.enable()
  controller.enable()
  controller.enable()

  assert.equal(doc.activeCount(), 1)
  doc.dispatchKey('Escape')
  assert.equal(calls, 1)
})

test('document keydown controller removes listeners on disable and dispose', () => {
  const doc = createDocumentStub()
  let calls = 0
  const controller = createDocumentKeydownController({
    documentRef: () => doc,
    key: 'Escape',
    onKey: () => { calls += 1 },
  })

  controller.enable()
  controller.disable()
  doc.dispatchKey('Escape')
  assert.equal(calls, 0)
  assert.equal(doc.activeCount(), 0)

  controller.enable()
  controller.dispose()
  doc.dispatchKey('Escape')
  assert.equal(calls, 0)
  assert.equal(doc.activeCount(), 0)
})

test('document keydown controller ignores other keys and missing documents', () => {
  const doc = createDocumentStub()
  let calls = 0
  const controller = createDocumentKeydownController({
    documentRef: () => doc,
    key: 'Escape',
    onKey: () => { calls += 1 },
  })

  controller.enable()
  doc.dispatchKey('Enter')
  assert.equal(calls, 0)

  const noDocument = createDocumentKeydownController({
    documentRef: () => null,
    key: 'Escape',
    onKey: () => { calls += 1 },
  })
  noDocument.enable()
  noDocument.dispose()
  assert.equal(calls, 0)
})
