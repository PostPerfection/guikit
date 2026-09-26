// The Settings page component list in src/component-versions.js, driven headless
// with a fake document and a stubbed invoke. `node --test test/`.
import assert from 'node:assert/strict'
import test from 'node:test'

const COMPONENTS = [
  { name: 'DCP Wizard', version: '1.3.3' },
  { name: 'PostKit', version: '0.6.0' },
  { name: 'Grok', version: '20.4.11' },
  { name: 'FFmpeg', version: '8.1' },
  { name: 'mpv', version: '2.5' },
]
const FAILURE = 'no backend'

function fakeElement(tag) {
  return {
    tag,
    className: '',
    textContent: '',
    children: [],
    append(...children) {
      this.children.push(...children)
    },
    replaceChildren(...children) {
      this.children = children
    },
  }
}

const container = fakeElement('div')

globalThis.document = {
  getElementById: (id) => (id === 'component-versions' ? container : null),
  createElement: fakeElement,
}

const { loadComponentVersions } = await import('../src/component-versions.js')

test('a row per component in order, with its name then its version', async () => {
  const invocations = []
  await loadComponentVersions(async (command) => {
    invocations.push(command)
    return COMPONENTS
  })

  assert.deepEqual(invocations, ['component_versions'])
  const rows = container.children.map((row) => {
    assert.equal(row.tag, 'div')
    assert.equal(row.className, 'component-version')
    const [name, version] = row.children
    assert.equal(name.tag, 'span')
    assert.equal(version.tag, 'output')
    return { name: name.textContent, version: version.textContent }
  })
  assert.deepEqual(rows, COMPONENTS)
})

test('a failed invoke says the versions could not be read', async () => {
  await loadComponentVersions(async () => {
    throw new Error(FAILURE)
  })

  assert.match(container.textContent, /^Could not read component versions:/)
  assert.ok(container.textContent.includes(FAILURE))
})
