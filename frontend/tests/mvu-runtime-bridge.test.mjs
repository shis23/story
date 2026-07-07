import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { getTrustedMvuRuntimeMessage } from '../src/mvu-runtime-bridge.js'

function extractRuntimeShimScript() {
  const source = fs.readFileSync(new URL('../src/components/MvuJsRuntime.vue', import.meta.url), 'utf8')
  const match = source.match(/const SHIM_SCRIPT = `([\s\S]*?)`;/)
  assert.ok(match, 'MvuJsRuntime.vue should define SHIM_SCRIPT')
  return match[1]
}

function createElement() {
  return {
    style: {},
    children: [],
    innerHTML: '',
    textContent: '',
    appendChild(child) {
      this.children.push(child)
      return child
    },
    querySelector() {
      return null
    },
    querySelectorAll() {
      return []
    },
    setAttribute(name, value) {
      this[name] = value
    },
    getAttribute(name) {
      return this[name] ?? ''
    },
  }
}

function createShimSandbox() {
  const listeners = {}
  const postedMessages = []
  const document = {
    body: { appendChild() {} },
    head: { appendChild() {} },
    createElement,
    querySelector() {
      return null
    },
    querySelectorAll() {
      return []
    },
  }
  const window = {
    addEventListener(name, callback) {
      listeners[name] = callback
    },
    setTimeout,
    setInterval,
    console,
  }
  const parent = {
    postMessage(message, targetOrigin) {
      postedMessages.push({ message, targetOrigin })
    },
  }
  const sandbox = {
    parent,
    document,
    console,
    getComputedStyle: () => ({}),
    setTimeout,
    setInterval,
    clearTimeout,
    clearInterval,
  }
  Object.assign(sandbox, window)
  sandbox.window = sandbox
  vm.runInNewContext(extractRuntimeShimScript(), sandbox)
  return { listeners, postedMessages, window: sandbox }
}

function plain(value) {
  return JSON.parse(JSON.stringify(value))
}

test('accepts MVU runtime messages only from the owned iframe window', () => {
  const runtimeWindow = { postMessage() {} }
  const pluginWindow = { postMessage() {} }
  const message = { type: 'mvu:execute_result', request_id: 'req-1' }

  assert.equal(
    getTrustedMvuRuntimeMessage({ source: runtimeWindow, origin: 'null', data: message }, runtimeWindow),
    message,
  )
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: pluginWindow, origin: 'null', data: message }, runtimeWindow),
    null,
  )
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: runtimeWindow, origin: 'null', data: { type: 'sf:ready' } }, runtimeWindow),
    null,
  )
})

test('rejects MVU runtime messages before the iframe window is available', () => {
  assert.equal(
    getTrustedMvuRuntimeMessage({ source: {}, data: { type: 'mvu:ready' } }, null),
    null,
  )
})

test('MVU iframe shim captures direct variables assignments and _.set updates', () => {
  const { listeners, postedMessages } = createShimSandbox()
  assert.equal(postedMessages.at(-1).message.type, 'mvu:ready')

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-variables',
      variables: { hp: 80, mood: 'calm' },
      fragment_js: [
        'variables.hp = variables.hp - 39;',
        'variables.mvu_probe = variables.hp;',
        '_.set("mana", 7);',
        'triggerSlashTag("probe");',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.equal(result.request_id, 'req-variables')
  assert.deepEqual(plain(result.variable_updates), { hp: 41, mvu_probe: 41, mana: 7 })
  assert.deepEqual(plain(result.side_effects), ['probe'])
})

test('MVU iframe shim reports final variable state when _.set and assignments touch the same key', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-final-state',
      variables: { hp: 80, unchanged: 5 },
      fragment_js: [
        '_.set("hp", 41);',
        'variables.hp = 42;',
        '_.set("mana", 7);',
        'variables.mana = 8;',
        '_.set("unchanged", 1);',
        'variables.unchanged = 5;',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.equal(result.request_id, 'req-final-state')
  assert.deepEqual(plain(result.variable_updates), { hp: 42, mana: 8 })
})

test('MVU iframe shim does not expose runtime private bindings to user fragments', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-private-scope',
      variables: {},
      fragment_js: [
        'variables.resType = typeof RES;',
        'variables.rawTimerType = typeof _setTimeout;',
        'variables.timerListType = typeof TM;',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.deepEqual(plain(result.variable_updates), {
    resType: 'undefined',
    rawTimerType: 'undefined',
    timerListType: 'undefined',
  })
})

test('MVU iframe shim keeps legacy non-strict script semantics without exposing private bindings', () => {
  const { listeners, postedMessages, window } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-legacy-scope',
      variables: {},
      fragment_js: [
        'legacyGlobalProbe = 12;',
        'variables.topThisIsWindow = this === window;',
        'variables.privateStillHidden = typeof RES;',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.equal(window.legacyGlobalProbe, 12)
  assert.deepEqual(plain(result.variable_updates), {
    topThisIsWindow: true,
    privateStillHidden: 'undefined',
  })
})

test('MVU iframe shim supports TavernHelper-style setvar/getvar aliases', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-st-vars',
      variables: { hp: 80, mood: 'calm' },
      fragment_js: [
        'setvar("hp", getvar("hp") - 15);',
        'window.setvar("status.mood", window.getvar("mood", "neutral"));',
        'variables.hpFromAlias = getvar("hp");',
        'variables.defaultFromAlias = getvar("missing", "fallback");',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.equal(result.request_id, 'req-st-vars')
  assert.deepEqual(plain(result.variable_updates), {
    hp: 65,
    'status.mood': 'calm',
    hpFromAlias: 65,
    defaultFromAlias: 'fallback',
  })
})

test('MVU iframe shim preserves window globals created by status bar assets', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:load_assets',
      html: '<main id="card"></main>',
      js: [
        'window.StatusBarCompat = window.StatusBarCompat || { renders: 0 };',
        'window.StatusBarCompat.readMood = function() { return window.getvar("mood", "neutral"); };',
        'window.StatusBarCompat.render = function() {',
        '  this.renders += 1;',
        '  window.setvar("lastRenderMood", this.readMood());',
        '  return this.renders;',
        '};',
      ].join('\n'),
    },
  })

  const loaded = postedMessages.at(-1).message
  assert.equal(loaded.type, 'mvu:assets_loaded')
  assert.equal(loaded.error, undefined)

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-window-status',
      variables: { mood: 'focused' },
      fragment_js: [
        'variables.renderCount = window.StatusBarCompat.render();',
        'variables.renderMood = window.getvar("lastRenderMood");',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.deepEqual(plain(result.variable_updates), {
    lastRenderMood: 'focused',
    renderCount: 1,
    renderMood: 'focused',
  })
})

test('MVU iframe shim runs jquery ready fallbacks and blocks remote script loads', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:load_assets',
      html: '<main id="card"></main>',
      js: [
        '$(function() { setvar("readyShortcut", true); });',
        '$(document).ready(function() { setvar("readyDocument", true); });',
        '$.getScript("https://testingcf.jsdelivr.net/storyforge/status-bar.js")',
        '  .fail(function() { setvar("remoteScriptBlocked", true); })',
        '  .always(function(_data, status) { setvar("remoteScriptStatus", status); });',
        'variables.assetsContinued = true;',
      ].join('\n'),
    },
  })

  const loaded = postedMessages.at(-1).message
  assert.equal(loaded.type, 'mvu:assets_loaded')
  assert.equal(loaded.error, undefined)

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-jquery-fallbacks',
      variables: {},
      fragment_js: [
        'variables.readyShortcutSeen = getvar("readyShortcut");',
        'variables.readyDocumentSeen = getvar("readyDocument");',
        'variables.remoteScriptBlockedSeen = getvar("remoteScriptBlocked");',
        'variables.remoteScriptStatusSeen = getvar("remoteScriptStatus");',
        'variables.assetsContinuedSeen = variables.assetsContinued;',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.deepEqual(plain(result.variable_updates), {
    readyShortcutSeen: true,
    readyDocumentSeen: true,
    remoteScriptBlockedSeen: true,
    remoteScriptStatusSeen: 'error',
    assetsContinuedSeen: true,
  })
})

test('MVU iframe shim blocks remote jquery load without failing asset bootstrap', () => {
  const { listeners, postedMessages } = createShimSandbox()

  listeners.message({
    data: {
      type: 'mvu:load_assets',
      html: '<main id="card"></main>',
      js: [
        '$("body").load("https://testingcf.jsdelivr.net/storyforge/probe.html", function(_html, status) {',
        '  variables.remoteLoadStatus = status;',
        '});',
        'variables.assetsContinued = true;',
      ].join('\n'),
    },
  })

  const loaded = postedMessages.at(-1).message
  assert.equal(loaded.type, 'mvu:assets_loaded')
  assert.equal(loaded.error, undefined)

  listeners.message({
    data: {
      type: 'mvu:execute',
      request_id: 'req-blocked-load',
      variables: {},
      fragment_js: [
        'variables.remoteLoadStatusSeen = variables.remoteLoadStatus;',
        'variables.assetsContinuedSeen = variables.assetsContinued;',
      ].join('\n'),
    },
  })

  const result = postedMessages.at(-1).message
  assert.equal(result.type, 'mvu:execute_result')
  assert.deepEqual(plain(result.variable_updates), {
    remoteLoadStatusSeen: 'error',
    assetsContinuedSeen: true,
  })
})
