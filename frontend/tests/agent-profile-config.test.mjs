import test from 'node:test'
import assert from 'node:assert/strict'
import {
  cleanForSave,
  parseWhitelist,
  safeFileName,
} from '../src/utils/agentProfileConfig.js'

test('safeFileName removes unsafe path characters and keeps a usable fallback', () => {
  assert.equal(safeFileName('  bad\\/name:*?"<>|  '), 'bad-name-')
  assert.equal(safeFileName('   '), 'agent-profile')
  assert.equal(safeFileName('a'.repeat(90)), 'a'.repeat(80))
})

test('parseWhitelist distinguishes default, explicit empty, and comma or newline lists', () => {
  assert.equal(parseWhitelist(null), null)
  assert.equal(parseWhitelist(undefined), null)
  assert.deepEqual(parseWhitelist(' \n\t '), [])
  assert.deepEqual(
    parseWhitelist(' search_world_info, get_character\nemit_plan,\n , search_vectors '),
    ['search_world_info', 'get_character', 'emit_plan', 'search_vectors'],
  )
})

test('cleanForSave trims empty fields while preserving explicit whitelist arrays', () => {
  const cfg = {
    id: 'profile-1',
    name: 'Custom',
    description: '',
    source: 'UserCreated',
    max_concurrent_subagents: '0',
    tool_whitelistRaw: 'legacy-top-level',
    agent_configs: {
      Director: {
        model_override: '  gpt-4.1  ',
        max_tool_rounds: '3',
        tool_whitelistRaw: ' search_world_info, get_character\nemit_plan ',
      },
      Summarizer: {
        model_override: '   ',
        max_tool_rounds: '',
      },
      PostProcessor: {
        model_override: null,
        max_tool_rounds: null,
        tool_whitelistRaw: '',
      },
      'Subagent:*': {
        model_override: '',
        max_tool_rounds: '2',
        tool_whitelist: ['get_character'],
      },
    },
  }

  const cleaned = cleanForSave(cfg)

  assert.equal(cleaned.max_concurrent_subagents, 1)
  assert.equal('tool_whitelistRaw' in cleaned, false)
  assert.deepEqual(Object.keys(cleaned.agent_configs).sort(), ['Director', 'PostProcessor', 'Subagent:*'].sort())
  assert.deepEqual(cleaned.agent_configs.Director, {
    model_override: 'gpt-4.1',
    max_tool_rounds: 3,
    tool_whitelist: ['search_world_info', 'get_character', 'emit_plan'],
  })
  assert.deepEqual(cleaned.agent_configs.PostProcessor, {
    model_override: null,
    max_tool_rounds: null,
    tool_whitelist: [],
  })
  assert.deepEqual(cleaned.agent_configs['Subagent:*'], {
    model_override: null,
    max_tool_rounds: 2,
    tool_whitelist: ['get_character'],
  })
})
